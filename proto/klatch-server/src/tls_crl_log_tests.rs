//! The tests of the refusal log (#1632, #1689), split from `tls_crl.rs` to keep it under the line cap.

use super::*;

#[test]
fn a_line_is_due_once_per_interval_and_the_lock_is_free_afterwards() {
    let slot = Mutex::new(None);
    let t = Instant::now();
    assert!(due(&slot, t));
    assert!(!due(&slot, t + EVERY / 2));
    assert!(slot.try_lock().is_ok());
    assert!(due(&slot, t + EVERY));
}

fn revoked() -> std::io::Error {
    std::io::Error::other(rustls::Error::InvalidCertificate(
        rustls::CertificateError::Revoked,
    ))
}

/// #1632: the line is written with no lock held, so a stderr that blocks cannot hold every later refusal at the mutex.
/// The writer stands in for stderr and checks, while it runs, that the slot's lock is free. Kills the mutant that puts
/// back the code which held the lock across the write.
#[test]
fn a_refusal_is_written_with_no_lock_held_and_at_most_once_per_interval() {
    let slots: [Mutex<Option<Instant>>; 3] = [const { Mutex::new(None) }; 3];
    let t = Instant::now();
    let mut said = Vec::new();
    note_to(&revoked(), &slots, t, |text| {
        assert!(
            slots[0].try_lock().is_ok(),
            "the lock is held while writing"
        );
        said.push(text.to_string());
    });
    assert_eq!(said.len(), 1, "{said:?}");
    // a second one inside the interval is not written, and one after it is
    note_to(&revoked(), &slots, t + EVERY / 2, |t| said.push(t.into()));
    assert_eq!(said.len(), 1, "{said:?}");
    note_to(&revoked(), &slots, t + EVERY, |t| said.push(t.into()));
    assert_eq!(said.len(), 2, "{said:?}");
    // a failure that is not the revocation check says nothing
    note_to(&std::io::Error::other("x"), &slots, t + EVERY * 9, |_| {
        panic!("written")
    });
}
