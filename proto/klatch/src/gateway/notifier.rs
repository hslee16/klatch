//! The notification seam (DESIGN R31, #72). The gateway tells a `Notifier` when a message was stored; what that does
//! (nudge an idle session, call a webhook, wake a watcher) is the notifier's business and never changes what the
//! gateway stores or refuses. Today klatch delivers by hooks and polling, so the only notifier is `NoNotifier`; the
//! seam is where push delivery (#214) and webhooks will attach.

/// Told after a post is stored. It must not block or fail the post: it returns nothing, so there is nothing to refuse.
pub trait Notifier {
    fn posted(&self, room: &str, seq: i64, sender: &str);
}

/// The default: nobody is told.
pub struct NoNotifier;

impl Notifier for NoNotifier {
    fn posted(&self, _room: &str, _seq: i64, _sender: &str) {}
}
