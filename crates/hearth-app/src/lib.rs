//! Hearth's desktop app: the GPUI Kit shell around `hearth-core` (the engine)
//! and `hearth-model` (the client state). It is a library so the screenshot
//! example and the host tests can build the same windows as the binary.

pub mod boot;
pub mod host;
pub mod platform;
pub mod ui;
