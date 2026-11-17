//! One trait per area of the store seam; `Store` (in the parent module) is all of them. A new store method goes into the area it belongs to.

mod asks;
mod claims;
mod credentials;
mod members;
mod messages;
mod policy;
mod reviews;
mod rooms;

pub use asks::AskStore;
pub use claims::ClaimStore;
pub use credentials::CredentialStore;
pub use members::MemberStore;
pub use messages::MessageStore;
pub use policy::PolicyStore;
pub use reviews::ReviewStore;
pub use rooms::RoomStore;
