//! A plain request that can be cancelled while it is inside the backend (#1562), for the front ends that read their
//! input on a second thread (ACP, #1042). It is `Session::call` with the request named, so a hook can find it.

use super::methods::Call;
use super::{Backend, CancelHook, Outcome, Session};
use std::sync::Arc;

/// Whether `call` is a pure look: it reads and changes nothing, so telling the caller "cancelled" is true whether or not
/// it had already run. `who`, `claims` and `history` never change anything; `read` is pure only with `peek: true`
/// (without it the read moves the cursor, and a cancelled read that had run would have moved it). Everything else (a
/// post, a claim, a release, an answer, a join) has an effect, so it is never reported `Cancelled`: a cancel after the
/// effect happened would be a lie. The one predicate both `Session::call_cancellable` and `SharedBackend` use.
pub(crate) fn is_look(call: &Call) -> bool {
    match call.method.as_str() {
        "who" | "claims" | "history" => true,
        "read" => call.value.get("peek").and_then(serde_json::Value::as_bool) == Some(true),
        _ => false,
    }
}

impl<B: Backend + ?Sized> Session<'_, B> {
    /// `call`, as the request named `id`: it may be cancelled through [`Session::canceller`] with that id. Meets the
    /// same checks `call` meets. The caller makes `id` unique among the requests it has in flight.
    pub fn call_cancellable(&self, id: &str, method: &str, params: serde_json::Value) -> Outcome {
        let call = Call {
            id: id.into(),
            method: method.into(),
            value: params,
        };
        // Only a pure look is handed to the cancellable path; anything with an effect is a plain request (see `is_look`).
        if !is_look(&call) {
            return Outcome::Done(self.request(&mut Vec::new(), call));
        }
        self.gateway
            .handle_cancellable(self.who, call, self.harness)
    }

    /// The hook that cancels what `call_cancellable` has in flight.
    pub fn canceller(&self) -> Arc<dyn CancelHook> {
        self.gateway.canceller()
    }
}
