//! Event system for communication between components

use std::time::Duration;
use serde::{Deserialize, Serialize};
use crate::domain::{VideoId, PlaybackState, PlaybackMode, ScanProgress, TimelineSegment, GridScanResponse, VideoMetadata};

/// Commands sent from UI to backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiCommand {
    OpenVideo { path: String, mode: PlaybackMode },
    Play(VideoId),
    Pause(VideoId),
    Stop(VideoId),
    Seek(VideoId, Duration),
    SetVolume(VideoId, f32),
    SetPlaybackRate(VideoId, f32),
    ToggleFullscreen(VideoId),
    RequestScan(VideoId),
    CancelScan(VideoId),
    SkipSegment(VideoId, Duration, Duration),
    UpdatePreferences(crate::domain::UserPreferences),
    Shutdown,
}

/// Events sent from backend to UI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackendEvent {
    VideoOpened {
        video_id: VideoId,
        metadata: VideoMetadata,
    },
    VideoOpenFailed {
        path: String,
        error: String,
    },
    PlaybackStateChanged {
        video_id: VideoId,
        state: PlaybackState,
    },
    PositionUpdate {
        video_id: VideoId,
        position: Duration,
        duration: Duration,
    },
    VolumeChanged {
        video_id: VideoId,
        volume: f32,
    },
    ScanProgress(ScanProgress),
    ScanComplete {
        video_id: VideoId,
        segments: Vec<TimelineSegment>,
    },
    ScanError {
        video_id: VideoId,
        error: String,
    },
    NewScanSegment {
        video_id: VideoId,
        segment: TimelineSegment,
    },
    AutoSkipTriggered {
        video_id: VideoId,
        from: Duration,
        to: Duration,
    },
    FrameReady {
        video_id: VideoId,
        timestamp: Duration,
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
    Error {
        video_id: Option<VideoId>,
        message: String,
    },
}

/// Internal events for the scan worker
#[derive(Debug, Clone)]
pub enum ScanWorkerEvent {
    StartScan {
        video_id: VideoId,
        video_path: String,
        total_duration: Duration,
    },
    ProcessGrid {
        request: crate::domain::GridScanRequest,
    },
    GridResult {
        response: GridScanResponse,
    },
    GridError {
        video_id: VideoId,
        grid_index: u32,
        error: String,
    },
    Complete(VideoId),
    Cancel(VideoId),
}

/// Events for the video engine
#[derive(Debug, Clone)]
pub enum VideoEngineEvent {
    Initialize { video_id: VideoId, path: String },
    Play(VideoId),
    Pause(VideoId),
    Stop(VideoId),
    Seek(VideoId, Duration),
    SetVolume(VideoId, f32),
    SetRate(VideoId, f32),
    RequestFrame(VideoId, Duration),
    Shutdown(VideoId),
}

/// Video engine responses
#[derive(Debug, Clone)]
pub enum VideoEngineResponse {
    Initialized { video_id: VideoId, metadata: VideoMetadata },
    InitFailed { video_id: VideoId, error: String },
    FrameReady { video_id: VideoId, timestamp: Duration, data: Vec<u8>, width: u32, height: u32 },
    PositionUpdate { video_id: VideoId, position: Duration, duration: Duration },
    StateChanged { video_id: VideoId, state: PlaybackState },
    EndOfFile(VideoId),
    Error { video_id: VideoId, error: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_event_serialization() {
        let cmd = UiCommand::Play(crate::domain::VideoId::new());
        let json = serde_json::to_string(&cmd).unwrap();
        let decoded: UiCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded, UiCommand::Play(_)));
    }
}