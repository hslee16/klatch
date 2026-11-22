//! What wakes a caller and how it is shown: wait and wake state, mute, the delivery view, the change token.

use super::Direct;
use crate::domain::Message;
use crate::domain::Result;
use crate::gateway::SharedTransport;
use crate::gateway::shared::forward;
use crate::store::Store;
use std::sync::atomic::Ordering;

/// What wakes a caller and how it is shown: wait and wake state, mute, the delivery view, the change token.
pub trait Delivery {
    /// Unread messages a wait has not looked at yet; moves no cursor, refreshes presence.
    fn wait_poll(&self, room: &str) -> Result<Vec<Message>>;
    /// How many unread messages would wake the caller in a room (mute aware); moves no cursor.
    fn wakeable(&self, room: &str) -> Result<usize>;
    /// The wakeable count and the newest wakeable seq (0 when none).
    fn wake_summary(&self, room: &str) -> Result<(usize, i64)>;
    /// How many unread messages the caller's narrowed view holds back in a room, and the newest seq (#673).
    fn outside_view(&self, room: &str) -> Result<Option<(usize, i64)>>;
    /// Record that a watcher pointed at what is unread in the room; it stays unread and `wait` still sees it.
    fn wake_mark(&self, room: &str) -> Result<()>;
    /// A wait has looked at the room up to `seq`; the messages stay unread.
    fn wait_seen(&self, room: &str, seq: i64) -> Result<()>;
    /// The store's change token (#616): differs after any other connection commits. A waiter takes it before it polls
    /// and sleeps until it differs. The default has none, so a waiter polls on a timer.
    fn change_token(&self) -> Option<u64> {
        None
    }
    /// Mute or unmute a room for the caller: its hook then shows only messages that mention the caller.
    fn set_muted(&self, room: &str, muted: bool) -> Result<()>;
    /// The caller's delivery view: what hooks and the wake count show it (#186).
    fn delivery_view(&self) -> Result<crate::gateway::DeliveryView>;
    fn set_delivery_view(&self, view: crate::gateway::DeliveryView) -> Result<()>;
}

impl<S: Store> Delivery for Direct<S> {
    fn wait_poll(&self, room: &str) -> Result<Vec<Message>> {
        self.gateway.wait_poll(&self.principal, room)
    }
    fn wakeable(&self, room: &str) -> Result<usize> {
        self.gateway.wakeable(&self.principal, room)
    }
    fn wake_summary(&self, room: &str) -> Result<(usize, i64)> {
        self.gateway.wake_summary(&self.principal, room)
    }
    fn outside_view(&self, room: &str) -> Result<Option<(usize, i64)>> {
        self.gateway.outside_view(&self.principal, room)
    }
    fn wake_mark(&self, room: &str) -> Result<()> {
        self.gateway.wake_mark(&self.principal, room)
    }
    fn wait_seen(&self, room: &str, seq: i64) -> Result<()> {
        self.gateway.wait_seen(&self.principal, room, seq)
    }
    fn change_token(&self) -> Option<u64> {
        self.gateway.change_token()
    }
    fn set_muted(&self, room: &str, muted: bool) -> Result<()> {
        self.gateway.set_muted(&self.principal, room, muted)
    }
    fn delivery_view(&self) -> Result<crate::gateway::DeliveryView> {
        self.gateway.delivery_view(&self.principal)
    }
    fn set_delivery_view(&self, view: crate::gateway::DeliveryView) -> Result<()> {
        self.gateway.set_delivery_view(&self.principal, view)
    }
}

impl Delivery for SharedTransport {
    forward! {
        wait_poll(room: &str) -> Result<Vec<crate::domain::Message>>;
        wakeable(room: &str) -> Result<usize>;
        wake_summary(room: &str) -> Result<(usize, i64)>;
        outside_view(room: &str) -> Result<Option<(usize, i64)>>;
        wake_mark(room: &str) -> Result<()>;
        wait_seen(room: &str, seq: i64) -> Result<()>;
        set_muted(room: &str, muted: bool) -> Result<()>;
        delivery_view() -> Result<crate::gateway::DeliveryView>;
        set_delivery_view(view: crate::gateway::DeliveryView) -> Result<()>;
    }

    /// The store's own token (it moves when another process commits) joined with the count of posts made through the
    /// handles (the store's does not move for the owner's own commits). `None` when the store has none.
    fn change_token(&self) -> Option<u64> {
        let store = self
            .gateway
            .call((&self.principal, &None), |t| t.change_token())
            .ok()
            .flatten()?;
        let posts = self.gateway.posts.load(Ordering::SeqCst);
        Some(store.wrapping_add(posts.rotate_left(32)))
    }
}
