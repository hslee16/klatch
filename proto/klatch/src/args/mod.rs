//! The command line, one file per family of commands. `Cmd` in `main.rs` flattens these and keeps the commands
//! that belong to no family.

use crate::*;

mod agents;
mod messaging;
mod wait;
mod work;

pub(crate) use agents::AgentsCmd;
pub(crate) use messaging::MessagingCmd;
pub(crate) use wait::WaitCmd;
pub(crate) use work::WorkCmd;
