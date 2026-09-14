//! Presentation-mode ship map client (library root).
//!
//! - [`geo`]: Fix / Ship / Track / Trail model + smoothing (see CONTEXT.md).
//! - [`backend`]: v0 poll contract: `PollSource`, file replay, HTTP stub.

pub mod backend;
pub mod catalog;
pub mod clock;
pub mod command;
pub mod fleet;
pub mod groups;
pub mod geo;
pub mod land;
pub mod log;
pub mod map_render;
pub mod overlay;
pub mod sim;
