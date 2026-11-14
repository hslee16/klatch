//! A key in a file readable only by its owner: the fallback where there is no keystore, and the way unattended
//! workers are keyed (a path handed over by the service manager or the parent, never the key itself).

use crate::keys::source::check_name;
use crate::keys::{Error, KEY_LEN, Key, KeySource};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub struct FileKeys {
    dir: PathBuf,
}

impl FileKeys {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn path(&self, name: &str) -> Result<PathBuf, Error> {
        check_name(name)?;
        Ok(self.dir.join(format!("{name}.key")))
    }

    fn ensure_dir(&self) -> Result<(), Error> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error::Store(format!("{}: {e}", self.dir.display())))?;
        std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| Error::Store(format!("{}: {e}", self.dir.display())))
    }
}

impl KeySource for FileKeys {
    fn get(&self, name: &str) -> Result<Option<Key>, Error> {
        let path = self.path(name)?;
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::Store(format!("{}: {e}", path.display()))),
        };
        // a key another user can read is no longer secret: refuse it instead of using it
        if meta.mode() & 0o077 != 0 {
            return Err(Error::Bad(format!(
                "{} is readable by other users (mode {:o}); fix it with chmod 600",
                path.display(),
                meta.mode() & 0o777
            )));
        }
        let bytes =
            std::fs::read(&path).map_err(|e| Error::Store(format!("{}: {e}", path.display())))?;
        let arr: [u8; KEY_LEN] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| Error::Bad(format!("{} is not a {KEY_LEN}-byte key", path.display())))?;
        Ok(Some(Key::from_bytes(arr)))
    }

    fn put(&self, name: &str, key: &Key) -> Result<(), Error> {
        let path = self.path(name)?;
        self.ensure_dir()?;
        // written beside the target with the final mode from the first byte, then renamed over it, so there is no
        // moment with a readable file and a crash leaves either the old key or the new one
        let tmp = path.with_extension("key.tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| Error::Store(format!("{}: {e}", tmp.display())))?;
        f.write_all(key.expose())
            .and_then(|()| f.sync_all())
            .map_err(|e| Error::Store(format!("{}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| Error::Store(format!("{}: {e}", path.display())))
    }

    fn delete(&self, name: &str) -> Result<(), Error> {
        let path = self.path(name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Store(format!("{}: {e}", path.display()))),
        }
    }

    fn describe(&self) -> &'static str {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("klatch-keys-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_key_round_trips_and_a_missing_one_is_none() {
        let d = dir("rt");
        let src = FileKeys::new(&d);
        assert!(src.get("store").unwrap().is_none());
        let k = Key::generate().unwrap();
        src.put("store", &k).unwrap();
        assert_eq!(src.get("store").unwrap().unwrap(), k);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_directory_is_0700_and_the_file_0600() {
        let d = dir("modes");
        let src = FileKeys::new(&d);
        src.put("store", &Key::generate().unwrap()).unwrap();
        let dm = std::fs::metadata(&d).unwrap().mode() & 0o777;
        let fm = std::fs::metadata(d.join("store.key")).unwrap().mode() & 0o777;
        assert_eq!((dm, fm), (0o700, 0o600));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_key_file_other_users_can_read_is_refused() {
        let d = dir("loose");
        let src = FileKeys::new(&d);
        src.put("store", &Key::generate().unwrap()).unwrap();
        std::fs::set_permissions(d.join("store.key"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let err = src.get("store").unwrap_err().to_string();
        assert!(
            err.contains("readable by other users") && err.contains("chmod 600"),
            "{err}"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_file_that_is_not_a_key_is_refused_without_printing_it() {
        let d = dir("short");
        let src = FileKeys::new(&d);
        src.put("store", &Key::generate().unwrap()).unwrap();
        std::fs::write(d.join("store.key"), b"too short").unwrap();
        std::fs::set_permissions(d.join("store.key"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        let err = src.get("store").unwrap_err().to_string();
        assert!(
            err.contains("not a 32-byte key") && !err.contains("too short"),
            "{err}"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn replacing_a_key_leaves_no_temporary_file_and_deleting_removes_it() {
        let d = dir("rot");
        let src = FileKeys::new(&d);
        src.put("store", &Key::generate().unwrap()).unwrap();
        let new = Key::generate().unwrap();
        src.put("store", &new).unwrap();
        assert_eq!(src.get("store").unwrap().unwrap(), new);
        let names: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["store.key".to_string()]);
        src.delete("store").unwrap();
        assert!(src.get("store").unwrap().is_none());
        src.delete("store").unwrap();
        std::fs::remove_dir_all(&d).ok();
    }
}
