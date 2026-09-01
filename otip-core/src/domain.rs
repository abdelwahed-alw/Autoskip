//! Core domain types for the video player

use std::fmt;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Unique identifier for a video session
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VideoId(pub Uuid);

impl VideoId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for VideoId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for VideoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Playback mode selected by user at video start
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackMode {
    /// Wait for full scan before playing
    SafeMode,
    /// Start immediately, scan async in background
    InstantPlay,
    /// Auto-skip explicit segments on-the-fly
    AutoSkip,
}

impl Default for PlaybackMode {
    fn default() -> Self {
        Self::SafeMode
    }
}

impl fmt::Display for PlaybackMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SafeMode => write!(f, "Safe Mode"),
            Self::InstantPlay => write!(f, "Instant Play (Zero Trust)"),
            Self::AutoSkip => write!(f, "Auto-Skip"),
        }
    }
}

/// Current state of video playback
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Buffering,
    Seeking,
    Ended,
    Error,
}

/// Result of NSFW detection for a time segment
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanSegment {
    pub start_time: Duration,
    pub end_time: Duration,
    pub is_explicit: bool,
    pub confidence: f32,
    pub quadrant_flags: Option<QuadrantFlags>,
}

/// Flags for 2x2 grid quadrants (TL, TR, BL, BR)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuadrantFlags {
    pub top_left: bool,
    pub top_right: bool,
    pub bottom_left: bool,
    pub bottom_right: bool,
}

impl QuadrantFlags {
    pub const fn new() -> Self {
        Self {
            top_left: false,
            top_right: false,
            bottom_left: false,
            bottom_right: false,
        }
    }

    pub fn any_explicit(&self) -> bool {
        self.top_left || self.top_right || self.bottom_left || self.bottom_right
    }

    pub fn explicit_count(&self) -> u8 {
        (self.top_left as u8) + (self.top_right as u8) + (self.bottom_left as u8) + (self.bottom_right as u8)
    }

    pub fn from_quadrant_numbers(quadrants: &[u8]) -> Self {
        let mut flags = Self::new();
        for &q in quadrants {
            match q {
                1 => flags.top_left = true,
                2 => flags.top_right = true,
                3 => flags.bottom_left = true,
                4 => flags.bottom_right = true,
                _ => {}
            }
        }
        flags
    }
}

impl Default for QuadrantFlags {
    fn default() -> Self {
        Self::new()
    }
}

/// Visual indicator for timeline segments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelineSegmentType {
    Unknown,
    ScannedSafe,
    ExplicitContent,
    SkipZone,
}

/// A segment on the visual timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSegment {
    pub start_time: Duration,
    pub end_time: Duration,
    pub segment_type: TimelineSegmentType,
    pub scan_segment: Option<ScanSegment>,
}

/// Video thumbnail data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoThumbnail {
    pub data: Vec<u8>,          // PNG encoded thumbnail
    pub width: u32,
    pub height: u32,
}

impl VideoThumbnail {
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        Self { data, width, height }
    }
}

/// Video metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub id: VideoId,
    pub path: String,
    pub title: String,
    pub duration: Duration,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub codec: String,
    pub has_audio: bool,
    pub created_at: DateTime<Utc>,
    pub last_played: Option<DateTime<Utc>>,
    pub thumbnail: Option<VideoThumbnail>,
}

/// Scan progress information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub video_id: VideoId,
    pub scanned_duration: Duration,
    pub total_duration: Duration,
    pub segments_found: usize,
    pub explicit_segments: usize,
    pub current_position: Duration,
    pub is_complete: bool,
    pub error: Option<String>,
}

impl ScanProgress {
    pub fn progress_percent(&self) -> f32 {
        if self.total_duration.as_secs_f32() > 0.0 {
            (self.scanned_duration.as_secs_f32() / self.total_duration.as_secs_f32()) * 100.0
        } else {
            0.0
        }
    }
}

/// User preferences for the application
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreferences {
    pub default_playback_mode: PlaybackMode,
    pub auto_skip_enabled: bool,
    pub scan_ahead_seconds: u32,
    pub confidence_threshold: f32,
    pub api_key: Option<String>,
    pub hardware_acceleration: bool,
    pub theme: Theme,
    pub language: String,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            default_playback_mode: PlaybackMode::SafeMode,
            auto_skip_enabled: true,
            scan_ahead_seconds: 30,
            confidence_threshold: 0.7,
            api_key: None,
            hardware_acceleration: true,
            theme: Theme::System,
            language: "en".to_string(),
        }
    }
}

/// Application theme
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Light,
    Dark,
    System,
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Light => write!(f, "Light"),
            Self::Dark => write!(f, "Dark"),
            Self::System => write!(f, "System"),
        }
    }
}

/// Video file info for playlist
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFileInfo {
    pub id: VideoId,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: DateTime<Utc>,
    pub metadata: Option<VideoMetadata>,
}

/// AI scan request for a grid of frames
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridScanRequest {
    pub video_id: VideoId,
    pub grid_index: u32,
    pub start_time: Duration,
    pub frame_data: Vec<u8>,
    pub mime_type: String,
}

/// AI scan response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridScanResponse {
    pub video_id: VideoId,
    pub grid_index: u32,
    pub explicit_quadrants: Vec<u8>,
    pub confidence_scores: Vec<f32>,
    pub processed_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_quadrant_flags() {
        let flags = QuadrantFlags::from_quadrant_numbers(&[1, 3]);
        assert!(flags.top_left);
        assert!(!flags.top_right);
        assert!(flags.bottom_left);
        assert!(!flags.bottom_right);
        assert_eq!(flags.explicit_count(), 2);
    }

    #[test]
    fn test_scan_progress() {
        let progress = ScanProgress {
            video_id: VideoId::new(),
            scanned_duration: Duration::from_secs(50),
            total_duration: Duration::from_secs(100),
            segments_found: 10,
            explicit_segments: 2,
            current_position: Duration::from_secs(50),
            is_complete: false,
            error: None,
        };
        assert_eq!(progress.progress_percent(), 50.0);
    }
}
