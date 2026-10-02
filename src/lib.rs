//! Presentation-mode ship map client (library root).
//!
//! - [`geo`]: Fix / Ship / Track / Trail model + smoothing (see CONTEXT.md).
//! - [`gpuprobe`]: `--probe-gpu`, the graphics capability check.
//! - [`fx`]: the shader seam — additive halos, quality tiers.
//! - [`camera`]: the eased camera move on an operator recentre.
//! - [`chrome`]: island chrome — the floating-panel shell (see ADR-0014).
//! - [`backend`]: v0 poll contract: `PollSource`, file replay, HTTP stub.
//! - [`symbology`]: APP-6C symbol model + generated icon geometry. Leaf: depends on nothing.

pub mod assets;
pub mod backend;
pub mod camera;
pub mod catalog;
pub mod chrome;
pub mod clock;
pub mod command;
pub mod fleet;
pub mod fx;
pub mod gpuprobe;
pub mod groups;
pub mod geo;
pub mod land;
pub mod log;
pub mod map_render;
pub mod overlay;
pub mod paths;
pub mod sim;
pub mod store;
pub mod symbology;
