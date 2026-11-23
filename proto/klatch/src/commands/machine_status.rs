//! The hook side of the machine status (#1403): what a PostToolUse or Stop hook that carries `--machine-status` asks
//! the gateway to show. Only the fact that a tool call happened, or that a turn ended, is used; the tool's name, its
//! input and its output are never read here, so none of them can reach the status.

use super::*;
use klatch::domain::MachineClass;

/// A tool call arrived (PostToolUse) and the hook carries the consent: the member is working. The gateway writes it
/// only for the first call after idle and at most once a window. A failure never stops the hook.
pub(super) fn on_tool_use(t: &dyn Transport, consent: bool) {
    if consent {
        let _ = t.set_machine_status(MachineClass::Working.word());
    }
}

/// A turn ended (Stop) and the hook carries the consent: idle, or parked when background tasks or crons still run.
pub(super) fn on_stop(t: &dyn Transport, consent: bool, waiting: bool) {
    if consent {
        let class = if waiting {
            MachineClass::Parked
        } else {
            MachineClass::Idle
        };
        let _ = t.set_machine_status(class.word());
    }
}
