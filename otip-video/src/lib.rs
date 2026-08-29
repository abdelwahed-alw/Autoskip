//! Video engine abstraction layer

pub mod engine;
pub mod mpv_backend;
pub mod gstreamer_backend;
pub mod frame_extractor;

pub use engine::{VideoEngine, VideoEngineHandle, EngineType};
pub use frame_extractor::MpvFrameExtractor;

#[cfg(feature = "mpv")]
pub use mpv_backend::MpvEngine;

#[cfg(feature = "gstreamer")]
pub use gstreamer_backend::GStreamerEngine;