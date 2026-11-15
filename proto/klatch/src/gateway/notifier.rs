//! The notification seam (DESIGN R31, #72). The gateway tells a `Notifier` when a message was stored; what that does
//! (nudge an idle session, call a webhook, wake a watcher) is the notifier's business and never changes what the
//! gateway stores or refuses. Today klatch delivers by hooks and polling, so the only notifier is `NoNotifier`; the
//! seam is where push delivery (#214) and webhooks will attach.

/// Told after a post is stored. It must not block or fail the post: it returns nothing, so there is nothing to refuse.
/// What a notifier does is an attempt, never a delivery: a socket write succeeds for messages the receiver holds,
/// refuses or lets expire (RESEARCH-NOTES 47.2), so only the reader's cursor or hook shown-mark may say delivered
/// (#555). A notifier must not move either.
pub trait Notifier {
    fn posted(&self, room: &str, seq: i64, sender: &str);
}

/// The default: nobody is told.
pub struct NoNotifier;

impl Notifier for NoNotifier {
    fn posted(&self, _room: &str, _seq: i64, _sender: &str) {}
}
