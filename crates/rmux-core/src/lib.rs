//! Deterministic domain and application logic. Adapters own all I/O and timing.
#![forbid(unsafe_code)]
mod application;
pub mod layout;
mod session;
pub use application::{Controller, PaneHost};
pub use layout::{Axis, Layout, PaneId, Rect};
pub use session::{Effect, Event, Session};
