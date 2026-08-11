//! Core domain logic for gittles: GitHub API access, device-flow auth, and the
//! on-disk store. This crate is deliberately free of UI dependencies so that a
//! future TUI front-end remains possible.

pub mod auth;
pub mod github;
pub mod store;
