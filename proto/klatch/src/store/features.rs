//! What a store declares it keeps (#485). Many area-trait methods have a default that answers "nothing" so a backend can be
//! written a little at a time; the trap is a backend that never writes the rest and loses the feature without any error.
//! Each such group of methods is a [`Feature`]. A store must say which it keeps (`RoomStore::features`, required, no
//! default), the gateway refuses a store that does not declare them all, and the conformance suite runs the tests of
//! every declared feature (`conformance::features`), so a backend can neither skip a feature quietly nor claim one it
//! does not keep.

use crate::domain::{Error, Result};
use crate::store::Store;

/// A group of optional store methods the gateway relies on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// `note_session`, `session_of_member`, `member_of_session`.
    Sessions,
    /// Status, scope, observed attributes, parents, hook runs, permission modes and parking of members.
    MemberNotes,
    /// `set_muted`, `is_muted`.
    Mute,
    /// `prune_members`.
    Pruning,
    /// `wait_seen`, `wake_seen`, and the polls and hook read that use their positions.
    WaitPositions,
    /// `release_with`, `last_release`.
    ClaimNotes,
    /// `release_anchored`, `anchors_in_use`.
    Anchors,
    /// Member claim release and retiming, owners, ages, claim changes, wait cycles and deadlock notices.
    ClaimLifecycle,
    /// The ask area: open, lapsed, stale and escalated asks and the working rooms.
    Asks,
    /// The review queue.
    Reviews,
    /// Issued credentials.
    Credentials,
}

impl Feature {
    pub const ALL: [Feature; 11] = [
        Feature::Sessions,
        Feature::MemberNotes,
        Feature::Mute,
        Feature::Pruning,
        Feature::WaitPositions,
        Feature::ClaimNotes,
        Feature::Anchors,
        Feature::ClaimLifecycle,
        Feature::Asks,
        Feature::Reviews,
        Feature::Credentials,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Feature::Sessions => "sessions",
            Feature::MemberNotes => "member notes",
            Feature::Mute => "mute",
            Feature::Pruning => "pruning",
            Feature::WaitPositions => "wait positions",
            Feature::ClaimNotes => "claim notes",
            Feature::Anchors => "anchors",
            Feature::ClaimLifecycle => "claim lifecycle",
            Feature::Asks => "asks",
            Feature::Reviews => "reviews",
            Feature::Credentials => "credentials",
        }
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// The set of features a store declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Features(u16);

impl Features {
    pub fn none() -> Self {
        Self(0)
    }

    pub fn all() -> Self {
        Feature::ALL.iter().fold(Self::none(), |f, x| f.with(*x))
    }

    pub fn with(self, feature: Feature) -> Self {
        Self(self.0 | feature.bit())
    }

    pub fn contains(self, feature: Feature) -> bool {
        self.0 & feature.bit() != 0
    }

    pub fn iter(self) -> impl Iterator<Item = Feature> {
        Feature::ALL.into_iter().filter(move |f| self.contains(*f))
    }

    /// The features the gateway relies on that this set lacks.
    pub fn missing(self) -> Vec<Feature> {
        Feature::ALL
            .into_iter()
            .filter(|f| !self.contains(*f))
            .collect()
    }
}

/// An error naming every feature `store` does not declare, or `Ok` when it declares them all.
pub fn require_features<S: Store + ?Sized>(store: &S) -> Result<()> {
    let missing = store.features().missing();
    if missing.is_empty() {
        return Ok(());
    }
    let names: Vec<&str> = missing.iter().map(|f| f.name()).collect();
    Err(Error::Backend(format!(
        "this store does not declare these features: {}",
        names.join(", ")
    )))
}
