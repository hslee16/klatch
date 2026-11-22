//! Which family handles each admin command, decided once and checked by the compiler (#686). `Route::from` is the
//! only match over every `AdminCmd`; each family then matches its own enum, so a new command fails to compile until
//! it is routed and a family function can never be handed a command it has no arm for.

use super::{AdminCmd, RoomPolicy};
use std::path::PathBuf;

/// A command and the family that handles it.
pub(super) enum Route {
    /// Needs only the gateway.
    Gateway(Act),
    /// Acts on the key or the store file, so needs the keystore as well.
    Keystore(Keystore),
    /// The passphrase itself and the shell that holds its unlock window (#1233).
    Passphrase(Passphrase),
}

pub(super) enum Passphrase {
    /// Set or change it: asks for the old one itself, so it is the one act that runs before a proof.
    Init,
    /// Reset it with the recovery code: also before a proof, because the passphrase is what is forgotten.
    Reset,
    Window {
        secs: Option<i64>,
        off: bool,
    },
    Shell,
}

pub(super) enum Act {
    Room(RoomAct),
    Member(MemberAct),
    Role(RoleAct),
    Policy(PolicyAct),
    Scrub(ScrubAct),
    Listing(ListingAct),
}

pub(super) enum Keystore {
    Key(KeyAct),
    Archive(ArchiveAct),
}

pub(super) enum RoomAct {
    Pause,
    Resume,
    Close { room: String },
    Reopen { room: String },
    Label { room: String, label: String },
    Canary { room: String },
    CanaryRetire { room: String },
}

pub(super) enum MemberAct {
    Remove {
        member: String,
        room: Option<String>,
        all_rooms: bool,
    },
    Allow {
        member: String,
        room: Option<String>,
        all_rooms: bool,
    },
    Clear {
        member: String,
        label: String,
    },
    GrantRooms {
        member: String,
        off: bool,
    },
    Grantor {
        room: String,
        member: String,
        off: bool,
    },
    RoomClear {
        room: String,
        member: String,
        label: Option<String>,
    },
    SshKey {
        label: String,
        member: String,
        off: bool,
    },
}

pub(super) enum RoleAct {
    Librarian { member: String, off: bool },
    Reviewer { member: String, off: bool },
    Facilitator { member: String, off: bool },
    Integrator { member: String, off: bool },
    AdminAgent { member: String, off: bool },
}

pub(super) enum PolicyAct {
    RoomCreation {
        policy: RoomPolicy,
    },
    RateLimit {
        posts: Option<usize>,
        secs: Option<i64>,
        off: bool,
        list: bool,
        member: Option<String>,
    },
    LeaseCap {
        secs: Option<i64>,
        off: bool,
    },
    LeaseGrace {
        secs: Option<i64>,
        off: bool,
    },
    RoomStale {
        room: String,
        secs: Option<i64>,
        off: bool,
    },
    RoomAskLease {
        room: String,
        secs: Option<i64>,
        off: bool,
    },
    RoomRequire {
        room: String,
        spec: Option<String>,
        off: bool,
    },
    Cap {
        label: String,
    },
    Chain {
        levels: String,
    },
}

pub(super) enum ScrubAct {
    Add { name: String, term: Vec<String> },
    Remove { name: String },
    List,
}

pub(super) enum ListingAct {
    Refusals { limit: usize },
    Audit { limit: usize },
    ReviewQueue { limit: usize },
}

pub(super) enum KeyAct {
    Recovery,
    Recover {
        code: Vec<String>,
        label: Option<String>,
    },
    SealLabel {
        label: String,
        keep_plain: bool,
    },
    RotateKey,
    RotateLabelKey {
        label: String,
    },
    Backup {
        file: PathBuf,
    },
    Restore {
        file: PathBuf,
    },
    RunArchive {
        file: PathBuf,
        run: String,
    },
    Export {
        room: String,
        json: bool,
    },
}

pub(super) enum ArchiveAct {
    Retention {
        room: String,
        days: String,
    },
    Archive {
        room: Option<String>,
        all_rooms: bool,
    },
    RestoreArchive {
        file: PathBuf,
    },
    Archives,
}

impl From<AdminCmd> for Route {
    fn from(cmd: AdminCmd) -> Self {
        use AdminCmd as C;
        match cmd {
            C::Pause => Route::room(RoomAct::Pause),
            C::Resume => Route::room(RoomAct::Resume),
            C::Close { room } => Route::room(RoomAct::Close { room }),
            C::Reopen { room } => Route::room(RoomAct::Reopen { room }),
            C::Label { room, label } => Route::room(RoomAct::Label { room, label }),
            C::Canary { room } => Route::room(RoomAct::Canary { room }),
            C::CanaryRetire { room } => Route::room(RoomAct::CanaryRetire { room }),
            C::Remove {
                member,
                room,
                all_rooms,
            } => Route::member(MemberAct::Remove {
                member,
                room,
                all_rooms,
            }),
            C::Allow {
                member,
                room,
                all_rooms,
            } => Route::member(MemberAct::Allow {
                member,
                room,
                all_rooms,
            }),
            C::Clear { member, label } => Route::member(MemberAct::Clear { member, label }),
            C::GrantRooms { member, off } => Route::member(MemberAct::GrantRooms { member, off }),
            C::Grantor { room, member, off } => {
                Route::member(MemberAct::Grantor { room, member, off })
            }
            C::RoomClear {
                room,
                member,
                label,
                off: _,
            } => Route::member(MemberAct::RoomClear {
                room,
                member,
                label,
            }),
            C::SshKey { label, member, off } => {
                Route::member(MemberAct::SshKey { label, member, off })
            }
            C::Librarian { member, off } => Route::role(RoleAct::Librarian { member, off }),
            C::Reviewer { member, off } => Route::role(RoleAct::Reviewer { member, off }),
            C::Facilitator { member, off } => Route::role(RoleAct::Facilitator { member, off }),
            C::Integrator { member, off } => Route::role(RoleAct::Integrator { member, off }),
            C::AdminAgent { member, off } => Route::role(RoleAct::AdminAgent { member, off }),
            C::RoomCreation { policy } => Route::policy(PolicyAct::RoomCreation { policy }),
            C::RateLimit {
                posts,
                secs,
                off,
                list,
                member,
            } => Route::policy(PolicyAct::RateLimit {
                posts,
                secs,
                off,
                list,
                member,
            }),
            C::LeaseCap { secs, off } => Route::policy(PolicyAct::LeaseCap { secs, off }),
            C::LeaseGrace { secs, off } => Route::policy(PolicyAct::LeaseGrace { secs, off }),
            C::RoomStale { room, secs, off } => {
                Route::policy(PolicyAct::RoomStale { room, secs, off })
            }
            C::RoomAskLease { room, secs, off } => {
                Route::policy(PolicyAct::RoomAskLease { room, secs, off })
            }
            C::RoomRequire { room, spec, off } => {
                Route::policy(PolicyAct::RoomRequire { room, spec, off })
            }
            C::Cap { label } => Route::policy(PolicyAct::Cap { label }),
            C::Chain { levels } => Route::policy(PolicyAct::Chain { levels }),
            C::ScrubAdd { name, term } => Route::scrub(ScrubAct::Add { name, term }),
            C::ScrubRemove { name } => Route::scrub(ScrubAct::Remove { name }),
            C::ScrubList => Route::scrub(ScrubAct::List),
            C::InitPassphrase => Route::Passphrase(Passphrase::Init),
            C::ResetPassphrase => Route::Passphrase(Passphrase::Reset),
            C::UnlockWindow { secs, off } => Route::Passphrase(Passphrase::Window { secs, off }),
            C::Shell => Route::Passphrase(Passphrase::Shell),
            C::Refusals { limit } => Route::listing(ListingAct::Refusals { limit }),
            C::Audit { limit } => Route::listing(ListingAct::Audit { limit }),
            C::ReviewQueue { limit } => Route::listing(ListingAct::ReviewQueue { limit }),
            C::Recovery => Route::key(KeyAct::Recovery),
            C::Recover { code, label } => Route::key(KeyAct::Recover { code, label }),
            C::SealLabel { label, keep_plain } => {
                Route::key(KeyAct::SealLabel { label, keep_plain })
            }
            C::RotateKey => Route::key(KeyAct::RotateKey),
            C::RotateLabelKey { label } => Route::key(KeyAct::RotateLabelKey { label }),
            C::Backup { file } => Route::key(KeyAct::Backup { file }),
            C::Restore { file } => Route::key(KeyAct::Restore { file }),
            C::RunArchive { file, run } => Route::key(KeyAct::RunArchive { file, run }),
            C::Export { room, json } => Route::key(KeyAct::Export { room, json }),
            C::Retention { room, days } => Route::archive(ArchiveAct::Retention { room, days }),
            C::Archive { room, all_rooms } => {
                Route::archive(ArchiveAct::Archive { room, all_rooms })
            }
            C::RestoreArchive { file } => Route::archive(ArchiveAct::RestoreArchive { file }),
            C::Archives => Route::archive(ArchiveAct::Archives),
        }
    }
}

impl Route {
    fn room(a: RoomAct) -> Self {
        Route::Gateway(Act::Room(a))
    }
    fn member(a: MemberAct) -> Self {
        Route::Gateway(Act::Member(a))
    }
    fn role(a: RoleAct) -> Self {
        Route::Gateway(Act::Role(a))
    }
    fn policy(a: PolicyAct) -> Self {
        Route::Gateway(Act::Policy(a))
    }
    fn scrub(a: ScrubAct) -> Self {
        Route::Gateway(Act::Scrub(a))
    }
    fn listing(a: ListingAct) -> Self {
        Route::Gateway(Act::Listing(a))
    }
    fn key(a: KeyAct) -> Self {
        Route::Keystore(Keystore::Key(a))
    }
    fn archive(a: ArchiveAct) -> Self {
        Route::Keystore(Keystore::Archive(a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::AdminCmd;

    fn family(cmd: AdminCmd) -> &'static str {
        match Route::from(cmd) {
            Route::Gateway(Act::Room(_)) => "room",
            Route::Gateway(Act::Member(_)) => "member",
            Route::Gateway(Act::Role(_)) => "role",
            Route::Gateway(Act::Policy(_)) => "policy",
            Route::Gateway(Act::Scrub(_)) => "scrub",
            Route::Gateway(Act::Listing(_)) => "listing",
            Route::Keystore(Keystore::Key(_)) => "key",
            Route::Keystore(Keystore::Archive(_)) => "archive",
            Route::Passphrase(_) => "passphrase",
        }
    }

    fn room(name: &str) -> String {
        name.to_string()
    }

    #[test]
    fn room_commands_route_to_the_room_family() {
        assert_eq!(family(AdminCmd::Pause), "room");
        assert_eq!(family(AdminCmd::Resume), "room");
        assert_eq!(family(AdminCmd::Close { room: room("r") }), "room");
        assert_eq!(family(AdminCmd::Reopen { room: room("r") }), "room");
        assert_eq!(family(AdminCmd::Canary { room: room("r") }), "room");
    }

    #[test]
    fn member_and_role_commands_route_apart() {
        let remove = AdminCmd::Remove {
            member: "m".into(),
            room: Some(room("r")),
            all_rooms: false,
        };
        assert_eq!(family(remove), "member");
        let librarian = AdminCmd::Librarian {
            member: "m".into(),
            off: false,
        };
        assert_eq!(family(librarian), "role");
        let facilitator = AdminCmd::Facilitator {
            member: "m".into(),
            off: false,
        };
        assert_eq!(family(facilitator), "role");
        let admin = AdminCmd::AdminAgent {
            member: "m".into(),
            off: true,
        };
        assert_eq!(family(admin), "role");
    }

    #[test]
    fn policy_scrub_and_listing_commands_route_to_their_families() {
        assert_eq!(family(AdminCmd::Cap { label: "l".into() }), "policy");
        assert_eq!(family(AdminCmd::ScrubList), "scrub");
        assert_eq!(family(AdminCmd::Audit { limit: 1 }), "listing");
        assert_eq!(family(AdminCmd::Refusals { limit: 1 }), "listing");
    }

    #[test]
    fn keystore_commands_split_into_key_and_archive() {
        assert_eq!(family(AdminCmd::RotateKey), "key");
        assert_eq!(family(AdminCmd::Recovery), "key");
        assert_eq!(family(AdminCmd::Archives), "archive");
        let archive = AdminCmd::Archive {
            room: None,
            all_rooms: true,
        };
        assert_eq!(family(archive), "archive");
    }
}
