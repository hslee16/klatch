//! The macOS Keychain, now only the place an older install kept its key (see `FileFirst`). A Keychain item is tied
//! to the program that made it, and the Keychain also keeps a partition list keyed to that program's code hash, so
//! a rebuilt unsigned program is a stranger and the Keychain asks for the login password (#480). Allowing every
//! program on the item (the `security` tool's `-A`) does not lift the partition list; only a stable signing
//! identity does (#371, #424). So a read here is time-limited: a hook must fail in a few seconds with the reason,
//! never wait for a dialog nobody answers.

use crate::keys::source::check_name;
use crate::keys::{Error, KEY_LEN, Key, KeySource};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

/// How long a Keychain read may take before it is given up as waiting for a dialog.
const READ_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

use crate::sync::within;

/// "item not found" in the Security framework.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

pub struct Keychain {
    service: String,
}

impl Keychain {
    /// Keys are generic passwords under this service name, one account per key name.
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }
}

impl KeySource for Keychain {
    fn get(&self, name: &str) -> Result<Option<Key>, Error> {
        check_name(name)?;
        let (service, account) = (self.service.clone(), name.to_string());
        let read = within(READ_LIMIT, move || get_generic_password(&service, &account));
        let Some(read) = read else {
            return Err(Error::Store(format!(
                "keychain: no answer in {} s; macOS is probably waiting for a password dialog. Answer it with Always \
                 Allow, or run klatch from the build that stored the key (see UNINSTALL.md)",
                READ_LIMIT.as_secs()
            )));
        };
        match read {
            Ok(bytes) => {
                let arr: [u8; KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                    Error::Bad(format!("keychain item {name} is not a {KEY_LEN}-byte key"))
                })?;
                Ok(Some(Key::from_bytes(arr)))
            }
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(Error::Store(format!("keychain: {e}"))),
        }
    }

    fn put(&self, name: &str, key: &Key) -> Result<(), Error> {
        check_name(name)?;
        set_generic_password(&self.service, name, key.expose())
            .map_err(|e| Error::Store(format!("keychain: {e}")))
    }

    fn delete(&self, name: &str) -> Result<(), Error> {
        check_name(name)?;
        match delete_generic_password(&self.service, name) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(e) => Err(Error::Store(format!("keychain: {e}"))),
        }
    }

    fn describe(&self) -> &'static str {
        "keychain"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real login keychain under a throwaway service name and removes it again.
    #[test]
    fn a_key_round_trips_through_the_login_keychain() {
        let service = format!("klatch-test-{}", std::process::id());
        let kc = Keychain::new(&service);
        let key = Key::generate().unwrap();
        assert!(kc.get("store").unwrap().is_none());
        kc.put("store", &key).unwrap();
        assert_eq!(kc.get("store").unwrap().unwrap(), key);
        let newer = Key::generate().unwrap();
        kc.put("store", &newer).unwrap();
        assert_eq!(kc.get("store").unwrap().unwrap(), newer);
        kc.delete("store").unwrap();
        assert!(kc.get("store").unwrap().is_none());
        kc.delete("store").unwrap();
    }

    /// #723: the word an error names the key source by. It builds the source and asks for the word; no Keychain item
    /// is read, written or deleted.
    #[test]
    fn the_keychain_names_itself_in_messages() {
        assert_eq!(Keychain::new("klatch-describe-only").describe(), "keychain");
    }

    #[test]
    fn a_stuck_call_is_given_up_on_and_a_quick_one_returns() {
        let t = std::time::Instant::now();
        let stuck = within(std::time::Duration::from_millis(200), || {
            std::thread::sleep(std::time::Duration::from_secs(30));
            1
        });
        assert!(stuck.is_none());
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(within(std::time::Duration::from_secs(5), || 7), Some(7));
    }

    /// Prints how long a fetch takes; not an assertion, since machines differ. Run with `-- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_keychain_fetch_latency() {
        let kc = Keychain::new(&format!("klatch-bench-{}", std::process::id()));
        kc.put("store", &Key::generate().unwrap()).unwrap();
        let mut ms: Vec<f64> = (0..200)
            .map(|_| {
                let t = std::time::Instant::now();
                let _ = kc.get("store").unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        kc.delete("store").unwrap();
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("keychain get: p50 {:.3} ms p99 {:.3} ms", ms[100], ms[197]);
    }
}
