//! Video engine trait and common types

use std::time::Duration;
use async_trait::async_trait;
use otip_core::domain::{VideoId, VideoMetadata, PlaybackState};
use otip_core::error::{Result, OtipError, VideoError};
use otip_core::events::{VideoEngineEvent, VideoEngineResponse};
use image::DynamicImage;
use tokio::sync::mpsc;

/// Video engine trait - implemented by backends
#[async_trait]
pub trait VideoEngine: Send + Sync {
    /// Initialize the engine with a video file
    async fn initialize(&mut self, video_id: VideoId, path: &str) -> Result<VideoMetadata>;
    
    /// Start playback
    async fn play(&mut self, video_id: VideoId) -> Result<()>;
    
    /// Pause playback
    async fn pause(&mut self, video_id: VideoId) -> Result<()>;
    
    /// Stop playback
    async fn stop(&mut self, video_id: VideoId) -> Result<()>;
    
    /// Seek to position
    async fn seek(&mut self, video_id: VideoId, position: Duration) -> Result<()>;
    
    /// Set volume (0.0 - 1.0)
    async fn set_volume(&mut self, video_id: VideoId, volume: f32) -> Result<()>;
    
    /// Set playback rate
    async fn set_rate(&mut self, video_id: VideoId, rate: f32) -> Result<()>;
    
    /// Get current position and duration
    async fn get_position(&self, video_id: VideoId) -> Result<(Duration, Duration)>;
    
    /// Get current playback state
    async fn get_state(&self, video_id: VideoId) -> Result<PlaybackState>;
    
    /// Request a frame at specific timestamp (for scanning)
    async fn request_frame(&mut self, video_id: VideoId, timestamp: Duration) -> Result<DynamicImage>;
    
    /// Check if hardware acceleration is available
    fn hw_acceleration_available(&self) -> bool;
    
    /// Get engine type identifier
    fn engine_type(&self) -> EngineType;
    
    /// Shutdown the engine
    async fn shutdown(&mut self, video_id: VideoId) -> Result<()>;
}

/// Engine type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineType {
    Mpv,
    GStreamer,
}

/// Handle to a running video engine
pub struct VideoEngineHandle {
    pub video_id: VideoId,
    pub engine_type: EngineType,
    pub command_tx: mpsc::UnboundedSender<VideoEngineEvent>,
    pub response_rx: mpsc::UnboundedReceiver<VideoEngineResponse>,
}

/// Factory for creating video engines
pub struct VideoEngineFactory;

impl VideoEngineFactory {
    /// Create the best available engine
    pub fn create_best() -> Box<dyn VideoEngine> {
        #[cfg(feature = "gstreamer")]
        {
            if GStreamerEngine::is_available() {
                return Box::new(GStreamerEngine::new());
            }
        }
        
        #[cfg(feature = "mpv")]
        {
            if MpvEngine::is_available() {
                return Box::new(MpvEngine::new());
            }
        }
        
        panic!("No video engine available. Enable 'mpv' or 'gstreamer' feature.");
    }

    /// Create specific engine type
    pub fn create(engine_type: EngineType) -> Result<Box<dyn VideoEngine>> {
        match engine_type {
            EngineType::Mpv => {
                #[cfg(feature = "mpv")]
                {
                    Ok(Box::new(MpvEngine::new()))
                }
                #[cfg(not(feature = "mpv"))]
                {
                    Err(OtipError::Video(VideoError::InitFailed(
                        "MPV feature not enabled".to_string()
                    )))
                }
            }
            EngineType::GStreamer => {
                #[cfg(feature = "gstreamer")]
                {
                    Ok(Box::new(GStreamerEngine::new()))
                }
                #[cfg(not(feature = "gstreamer"))]
                {
                    Err(OtipError::Video(VideoError::InitFailed(
                        "GStreamer feature not enabled".to_string()
                    )))
                }
            }
        }
    }
}

/// Configuration for video engine
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub hw_acceleration: bool,
    pub preferred_hw_api: Option<String>,
    pub vo_driver: Option<String>, // video output driver
    pub ao_driver: Option<String>, // audio output driver
    pub frame_extraction_resolution: (u32, u32),
    pub cache_duration: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            hw_acceleration: true,
            preferred_hw_api: None,
            vo_driver: None,
            ao_driver: None,
            frame_extraction_resolution: (320, 180),
            cache_duration: Duration::from_secs(10),
        }
    }
}

#[cfg(feature = "gstreamer")]
pub mod gstreamer_engine {
    use super::*;
    use crate::gstreamer_backend::GStreamerEngine;

    impl GStreamerEngine {
        pub fn is_available() -> bool {
            gstreamer::init().is_ok()
        }
    }
}

#[cfg(feature = "gstreamer")]
pub use crate::gstreamer_backend::GStreamerEngine;

#[cfg(feature = "mpv")]
pub mod mpv_engine {
    use super::*;
    use crate::mpv_backend::MpvEngine;

    impl MpvEngine {
        pub fn is_available() -> bool {
            true
        }
    }
}

#[cfg(feature = "mpv")]
pub use crate::mpv_backend::MpvEngine;
