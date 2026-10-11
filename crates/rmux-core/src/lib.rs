//! Deterministic domain and application logic. Adapters own all I/O and timing.
#![forbid(unsafe_code)]
mod application;
pub mod layout;
mod session;
mod sync;
pub use application::{Controller, PaneHost};
pub use layout::{Axis, Direction, Layout, PaneChrome, PaneId, Rect};
pub use session::{Effect, Event, HARD_MAX_PANES, HARD_WRITE_LIMIT, Limits, Session};
pub use sync::{
    EchoPrediction, HARD_FRAME_LIMIT, HARD_PREDICTION_LIMIT, Observe, PacketFault, ScreenSync,
    SessionKey, Snapshot, SyncFault, open_packet,
};
