//! Core domain logic for gittles: GitHub API access, device-flow auth, and the
//! on-disk store. This crate is deliberately free of UI dependencies so that a
//! future TUI front-end remains possible.

pub mod auth;
pub mod github;
pub mod search;
pub mod store;

pub use auth::{DeviceCode, Poll};
pub use github::{GitHub, Star};
pub use store::{Config, Store};
