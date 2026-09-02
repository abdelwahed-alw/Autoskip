//! Video engine abstraction layer - libmpv render_context + wgpu zero-copy (VLC-killer)

pub mod engine;
pub mod frame_extractor;
pub mod mpv_backend;

pub use engine::{VideoEngine, VideoEngineHandle, EngineType};
pub use mpv_backend::MpvEngine;