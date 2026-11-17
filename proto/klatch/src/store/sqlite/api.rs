//! `Store` for SQLite, one module per area (the traits are in `store::area`). Each module holds only that area's trait impl; the
//! queries and helpers it leans on live in the sibling modules of `sqlite`.

mod asks;
mod claims;
mod credentials;
mod members;
mod messages;
mod policy;
mod reviews;
mod rooms;
