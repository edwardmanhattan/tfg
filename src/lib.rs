//! Presentation-mode ship map client (library root).
//!
//! - [`geo`]: Fix / Ship / Track / Trail model + smoothing (see CONTEXT.md).
//! - [`backend`]: v0 poll contract: `PollSource`, file replay, HTTP stub.

pub mod backend;
pub mod geo;
pub mod map_render;
