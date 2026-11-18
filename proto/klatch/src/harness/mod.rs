//! Which running coding-agent session is this process part of? Reads the harness registry and the process
//! table, never connects to anything, never reads a key file and never touches the environment, which holds
//! the session's messaging token (DESIGN section 20 rule 15, RESEARCH-NOTES section 17).
//!
//! This is same-user evidence: anything running as the user can read the registry, so a session found here
//! stays unattributed.

mod ancestry;
mod capability;
mod host;
mod liveness;
mod probe;
mod process;
mod pstime;
mod record;
mod selftest;
mod sessions;

pub use ancestry::{Found, Miss, find_parent, find_session};
pub use capability::{
    ASYNC_REWAKE_FLOOR, Support, Version, parse_version, probe_claude_version, wake_support,
    wake_support_here,
};
pub use host::SysHost;
pub use liveness::{Liveness, check};
pub use probe::{
    Evidence, FileMeta, Host, LAYERS, Probe, ProcRow, Sample, Status, Thresholds, Turn, Verdict,
    parse_turn, probe,
};
pub use process::{Lookup, ProcInfo, ProcessTable, PsTable};
pub use pstime::parse_cpu_time;
pub use record::{FsRegistry, Record, Registry, parse_record};
pub use selftest::{LayerReport, SelfTest, self_test};
pub use sessions::{Sessions, list};
