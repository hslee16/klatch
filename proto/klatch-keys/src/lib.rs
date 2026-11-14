//! Key custody (epic #152, #155): where the store's keys live and how they are made, recovered and removed.
//!
//! A key is 32 random bytes. It is kept by a [`KeySource`]: the operating-system keystore where there is one, a
//! 0600 file where there is not (and for unattended workers). It never goes in the environment, on a command line,
//! in a log or in a `Debug` print, and it is wiped from memory when dropped.

mod file;
mod key;
mod recovery;
mod source;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

pub use file::FileKeys;
pub use key::{KEY_LEN, Key};
pub use recovery::{RecoveryError, recovery_code, restore};
pub use source::{
    Error, KeySource, MemoryKeys, create, default_source, open_or_create, store_key_name,
};

#[cfg(target_os = "macos")]
pub use macos::Keychain;
