//! A plain request that can be cancelled while it is inside the backend (#1562), for the front ends that read their
//! input on a second thread (ACP, #1042). It is `Session::call` with the request named, so a hook can find it.

use super::methods::Call;
use super::{Backend, CancelHook, Outcome, Session};
use std::sync::Arc;

impl<B: Backend + ?Sized> Session<'_, B> {
    /// `call`, as the request named `id`: it may be cancelled through [`Session::canceller`] with that id. Meets the
    /// same checks `call` meets. The caller makes `id` unique among the requests it has in flight.
    pub fn call_cancellable(&self, id: &str, method: &str, params: serde_json::Value) -> Outcome {
        let call = Call {
            id: id.into(),
            method: method.into(),
            value: params,
        };
        match method {
            // These change the session's own subscriptions and do not wait on the backend.
            "subscribe" | "unsubscribe" => Outcome::Done(self.request(&mut Vec::new(), call)),
            _ => self
                .gateway
                .handle_cancellable(self.who, call, self.harness),
        }
    }

    /// The hook that cancels what `call_cancellable` has in flight.
    pub fn canceller(&self) -> Arc<dyn CancelHook> {
        self.gateway.canceller()
    }
}
