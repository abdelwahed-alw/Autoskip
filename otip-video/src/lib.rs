//! Video engine abstraction layer

pub mod engine;
pub mod gstreamer_backend;
pub mod frame_extractor;

pub use engine::{VideoEngine, VideoEngineHandle, EngineType};

#[cfg(feature = "gstreamer")]
pub use gstreamer_backend::GStreamerEngine;