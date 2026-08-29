//! Application state management

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use iced::widget::canvas::Cache;
use otip_core::domain::{
    VideoId, PlaybackMode, PlaybackState, VideoMetadata, TimelineSegment, 
    TimelineSegmentType, ScanProgress, UserPreferences, Theme, VideoFileInfo,
};
use otip_core::timeline::format_duration_short;
use otip_core::events::{UiCommand, BackendEvent};
use otip_core::timeline::Timeline;
use otip_video::engine::VideoEngine;
use otip_ai::moderator::ContentModerator;
use crate::widgets::timeline::TimelineWidget;

/// Main application state
pub struct AppState {
    // Video state
    pub current_video: Option<VideoId>,
    pub video_metadata: Option<VideoMetadata>,
    pub playback_state: PlaybackState,
    pub position: Duration,
    pub duration: Duration,
    pub volume: f32,
    pub playback_rate: f32,
    pub is_fullscreen: bool,
    
    // Playback mode selection
    pub pending_video_path: Option<PathBuf>,
    pub show_mode_selection: bool,
    pub selected_mode: PlaybackMode,
    
    // Timeline
    pub timeline: Option<Arc<Timeline>>,
    pub timeline_cache: Cache,
    pub timeline_widget: TimelineWidget,
    pub hover_position: Option<Duration>,
    pub is_seeking: bool,
    
    // Scan state
    pub scan_progress: Option<ScanProgress>,
    pub is_scanning: bool,
    pub scan_error: Option<String>,
    
    // UI state
    pub show_file_picker: bool,
    pub show_settings: bool,
    pub show_playlist: bool,
    pub playlist: Vec<VideoFileInfo>,
    pub playlist_index: Option<usize>,
    
    // Preferences
    pub preferences: UserPreferences,

    // Gemini settings (mirrors AppConfig, editable in Settings)
    pub gemini_api_key: String,
    pub gemini_model: String,
    pub gemini_api_key_visible: bool,
    
    // Channels for backend communication
    pub command_tx: mpsc::UnboundedSender<UiCommand>,
    pub event_rx: mpsc::UnboundedReceiver<BackendEvent>,
    
    // Engine and moderator (wrapped in Arc for sharing)
    pub video_engine: Option<Arc<tokio::sync::Mutex<Box<dyn VideoEngine>>>>,
    pub content_moderator: Option<Arc<ContentModerator>>,
    
    // Window state
    pub window_width: f32,
    pub window_height: f32,
}

impl AppState {
    pub fn new(
        command_tx: mpsc::UnboundedSender<UiCommand>,
        event_rx: mpsc::UnboundedReceiver<BackendEvent>,
        preferences: UserPreferences,
    ) -> Self {
        Self {
            current_video: None,
            video_metadata: None,
            playback_state: PlaybackState::Stopped,
            position: Duration::ZERO,
            duration: Duration::ZERO,
            volume: 0.7,
            playback_rate: 1.0,
            is_fullscreen: false,
            
            pending_video_path: None,
            show_mode_selection: false,
            selected_mode: preferences.default_playback_mode,
            
            timeline: None,
            timeline_cache: Cache::default(),
            timeline_widget: TimelineWidget::new(),
            hover_position: None,
            is_seeking: false,
            
            scan_progress: None,
            is_scanning: false,
            scan_error: None,
            
            show_file_picker: false,
            show_settings: false,
            show_playlist: false,
            playlist: Vec::new(),
            playlist_index: None,
            
            preferences: preferences.clone(),

            gemini_api_key: String::new(),
            gemini_model: otip_core::config::GEMINI_DEFAULT_MODEL.to_string(),
            gemini_api_key_visible: false,
            
            command_tx,
            event_rx,
            
            video_engine: None,
            content_moderator: None,
            
            window_width: 1280.0,
            window_height: 720.0,
        }
    }

    /// Initialize Gemini config from loaded AppConfig
    pub fn set_gemini_config(&mut self, api_key: Option<String>, model: String) {
        self.gemini_api_key = api_key.unwrap_or_default();
        self.gemini_model = model;
    }

    /// Get masked API key for display (show last 4 chars if not visible)
    pub fn masked_api_key(&self) -> String {
        if self.gemini_api_key.is_empty() {
            return String::new();
        }
        if self.gemini_api_key_visible {
            self.gemini_api_key.clone()
        } else if self.gemini_api_key.len() > 8 {
            format!("{}…{}", &self.gemini_api_key[..4], &self.gemini_api_key[self.gemini_api_key.len()-4..])
        } else {
            "••••••••".to_string()
        }
    }

    /// Set video engine
    pub fn set_video_engine(&mut self, engine: Box<dyn VideoEngine>) {
        self.video_engine = Some(Arc::new(tokio::sync::Mutex::new(engine)));
    }

    /// Set content moderator
    pub fn set_content_moderator(&mut self, moderator: ContentModerator) {
        self.content_moderator = Some(Arc::new(moderator));
    }

    /// Open a video file
    pub fn open_video(&mut self, path: PathBuf) {
        self.pending_video_path = Some(path);
        self.show_mode_selection = true;
        self.selected_mode = self.preferences.default_playback_mode;
    }

    /// Confirm mode selection and start video
    pub fn confirm_mode_selection(&mut self) {
        if let Some(path) = self.pending_video_path.take() {
            self.show_mode_selection = false;
            let _ = self.command_tx.send(UiCommand::OpenVideo {
                path: path.to_string_lossy().to_string(),
                mode: self.selected_mode,
            });
        }
    }

    /// Cancel mode selection
    pub fn cancel_mode_selection(&mut self) {
        self.pending_video_path = None;
        self.show_mode_selection = false;
    }

    /// Update playback position
    pub fn update_position(&mut self, position: Duration, duration: Duration) {
        self.position = position;
        self.duration = duration;
        
        // Check for auto-skip
        if let Some(timeline) = &self.timeline {
            if self.preferences.auto_skip_enabled && !self.is_seeking {
                if let Some(next_safe) = timeline.get_next_safe_position(position, self.preferences.scan_ahead_seconds * 1000 / 30) {
                    // Trigger auto-skip
                    let _ = self.command_tx.send(UiCommand::SkipSegment(
                        self.current_video.unwrap(),
                        position,
                        next_safe,
                    ));
                }
            }
        }
    }

    /// Update playback state
    pub fn update_playback_state(&mut self, state: PlaybackState) {
        self.playback_state = state;
    }

    /// Update scan progress
    pub fn update_scan_progress(&mut self, progress: ScanProgress) {
        self.scan_progress = Some(progress.clone());
        self.is_scanning = !progress.is_complete;
        
        if progress.is_complete {
            self.is_scanning = false;
        }
    }

    /// Add timeline segment
    pub fn add_timeline_segment(&mut self, segment: TimelineSegment) {
        if let Some(timeline) = &self.timeline {
            timeline.add_segment(segment);
            self.timeline_cache.clear();
        }
    }

    /// Set timeline
    pub fn set_timeline(&mut self, timeline: Arc<Timeline>) {
        self.timeline = Some(timeline);
        self.timeline_cache.clear();
    }

    /// Toggle play/pause
    pub fn toggle_playback(&mut self) {
        if let Some(video_id) = self.current_video {
            let _ = match self.playback_state {
                PlaybackState::Playing => self.command_tx.send(UiCommand::Pause(video_id)),
                _ => self.command_tx.send(UiCommand::Play(video_id)),
            };
        }
    }

    /// Seek to position
    pub fn seek(&mut self, position: Duration) {
        if let Some(video_id) = self.current_video {
            self.is_seeking = true;
            let _ = self.command_tx.send(UiCommand::Seek(video_id, position));
        }
    }

    /// Set volume
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(video_id) = self.current_video {
            let _ = self.command_tx.send(UiCommand::SetVolume(video_id, self.volume));
        }
    }

    /// Set playback rate
    pub fn set_playback_rate(&mut self, rate: f32) {
        self.playback_rate = rate.clamp(0.25, 4.0);
        if let Some(video_id) = self.current_video {
            let _ = self.command_tx.send(UiCommand::SetPlaybackRate(video_id, self.playback_rate));
        }
    }

    /// Toggle fullscreen
    pub fn toggle_fullscreen(&mut self) {
        self.is_fullscreen = !self.is_fullscreen;
        if let Some(video_id) = self.current_video {
            let _ = self.command_tx.send(UiCommand::ToggleFullscreen(video_id));
        }
    }

    /// Format duration as MM:SS or HH:MM:SS
    pub fn format_duration(duration: Duration) -> String {
        let total_seconds = duration.as_secs();
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;
        
        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("{:02}:{:02}", minutes, seconds)
        }
    }

    /// Get progress as 0.0-1.0
    pub fn progress(&self) -> f32 {
        if self.duration.as_secs_f32() > 0.0 {
            (self.position.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Get scan progress as 0.0-1.0
    pub fn scan_progress_percent(&self) -> f32 {
        self.scan_progress.as_ref().map(|p| p.progress_percent()).unwrap_or(0.0)
    }
}

/// Messages for the app
#[derive(Debug, Clone)]
pub enum Message {
    // Video control
    PlayPause,
    Stop,
    Seek(f32), // 0.0-1.0
    SeekTo(Duration),
    VolumeChanged(f32),
    PlaybackRateChanged(f32),
    ToggleFullscreen,
    
    // File operations
    OpenFile,
    FileSelected(Option<PathBuf>),
    
    // Mode selection
    ModeSelected(PlaybackMode),
    ConfirmMode,
    CancelMode,
    
    // Scan events
    ScanProgress(ScanProgress),
    ScanComplete(Vec<TimelineSegment>),
    ScanError(String),
    NewScanSegment(TimelineSegment),
    
    // Playback events
    PositionUpdate(Duration, Duration),
    StateChanged(PlaybackState),
    VideoOpened(VideoId, VideoMetadata),
    VideoOpenFailed(String),
    AutoSkipTriggered(Duration, Duration),
    
    // UI
    ShowSettings(bool),
    ShowPlaylist(bool),
    TogglePlaylist,
    WindowResized(f32, f32),
    TimelineHover(Option<Duration>),
    TimelineSeekStart,
    TimelineSeekEnd(Duration),
    
    // Preferences
    PreferencesChanged(UserPreferences),

    // Gemini config
    ApiKeyChanged(String),
    ApiKeyVisibilityToggled,
    GeminiModelChanged(String),
    SaveGeminiConfig,
    GeminiConfigSaved(Result<(), String>),
    
    // Tick for animation
    Tick,
    
    // Backend event
    BackendEvent(BackendEvent),
    
    // Shutdown
    Shutdown,
}