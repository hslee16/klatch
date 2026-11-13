//! klatch-gateway: the single write path (DESIGN section 19 D6, section 20 rules 10 to 14). Every
//! operation takes a `Principal`, validates its inputs, passes the `Guard` exactly once, and only then
//! touches the store. The label check will live in the guard and nowhere else.

mod clock;
mod gateway;
mod guard;
mod subscription;
mod transport;

pub use clock::{Clock, FixedClock, SystemClock};
pub use gateway::{Digest, Gateway, Refused};
pub use guard::{AllowAll, Guard, Operation};
pub use subscription::{Batch, DEFAULT_WINDOW, Subscription};
pub use transport::{Direct, Transport};

#[cfg(test)]
mod test_support;
