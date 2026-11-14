//! The macOS Keychain. Reading goes through the Security framework. Writing goes through the `security` tool in its
//! interactive mode, with the command on standard input, so the key is never on a command line.
//!
//! The item is created with "allow all applications". A Keychain item is otherwise tied to the program that made it,
//! and an unsigned program that is rebuilt is a different program every time: each rebuild would make the Keychain
//! ask, and a hook has nobody to answer (measured: a different binary reading such an item hung). This is the same
//! protection as a key file readable only by you, against another user or a stolen disk; it does not stop another
//! program of yours. Signing klatch with a stable identity and binding the item to it is the later step
//! (DESIGN section 32 item 3).

use crate::keys::source::check_name;
use crate::keys::{Error, KEY_LEN, Key, KeySource};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

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
        match get_generic_password(&self.service, name) {
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
