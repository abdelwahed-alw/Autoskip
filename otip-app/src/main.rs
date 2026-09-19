//! Otip - Iced 0.14 with 3-screen routing + GStreamer playbin rendering
//! Splash → Library (thumbnails) → Player (Image from playbin frames + controls)

mod video_player;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use iced::{
    widget::{button, column, container, image, mouse_area, pick_list, progress_bar, row, scrollable, slider, text, text_input, Space, stack},
    Alignment, Background, Border, Color, Element, Length, Shadow, Task, Theme,
    keyboard::{self, key::Named},
    mouse,
    Event,
    window,
};
use iced::widget::image::Handle;
use otip_core::domain::PlaybackMode;
use otip_core::timeline::format_duration_short;
use tracing_subscriber::EnvFilter;
use video_player::{ChapterInfo, PlayerEvent, RenderQuality, SubTrackInfo, VideoPlayerHandle};
use iced::futures::SinkExt;

/// YouTube/Spotify-style dark palette — single source of truth for UI chrome.
///
/// | Role              | Hex     | Usage                              |
/// |-------------------|---------|------------------------------------|
/// | Main background   | #121212 | full-screen background             |
/// | Elevated surface  | #212121 | menus, bars, cards                 |
/// | Secondary surface | #303030 | buttons, borders, dividers, hover  |
/// | Primary text      | #FFFFFF | headings, important text           |
/// | Secondary text    | #AAAAAA | descriptions, secondary text       |
/// | Accent            | #3EA6FF | play button, progress, active      |
/// | Alert/error       | #CF6679 | error messages only                |
///
/// (Swap `ACCENT` for `#1DB954` │ `(0.1137, 0.7255, 0.3294)` │ for the Spotify-green variant.)
pub mod palette {
    use iced::Color;

    pub const BG_MAIN: Color = Color { r: 0.0706, g: 0.0706, b: 0.0706, a: 1.0 }; // #121212
    pub const BG_ELEVATED: Color = Color { r: 0.1294, g: 0.1294, b: 0.1294, a: 1.0 }; // #212121
    pub const BG_HOVER: Color = Color { r: 0.1882, g: 0.1882, b: 0.1882, a: 1.0 }; // #303030
    pub const TEXT_MAIN: Color = Color::WHITE; // #FFFFFF
    pub const TEXT_DIM: Color = Color { r: 0.6667, g: 0.6667, b: 0.6667, a: 1.0 }; // #AAAAAA
    pub const ACCENT: Color = Color { r: 0.2431, g: 0.6510, b: 1.0, a: 1.0 }; // #3EA6FF
    pub const ACCENT_HOVER: Color = Color { r: 0.36, g: 0.72, b: 1.0, a: 1.0 };
    pub const ACCENT_PRESSED: Color = Color { r: 0.16, g: 0.52, b: 0.92, a: 1.0 };
    pub const ALERT: Color = Color { r: 0.8118, g: 0.4000, b: 0.4745, a: 1.0 }; // #CF6679

    // ── Additional design-system colors ──
    pub const ACCENT_DIM: Color = Color { r: 0.2431, g: 0.6510, b: 1.0, a: 0.18 }; // accent tint for badges/pills
    pub const SURFACE: Color = Color { r: 0.16, g: 0.16, b: 0.16, a: 1.0 }; // #292929 card surfaces
    pub const BORDER_SUBTLE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }; // very faint card edges
    pub const SUCCESS: Color = Color { r: 0.18, g: 0.80, b: 0.44, a: 1.0 }; // #2ECC71 green accents
    pub const SUCCESS_DIM: Color = Color { r: 0.18, g: 0.80, b: 0.44, a: 0.18 };
    pub const WARN: Color = Color { r: 1.0, g: 0.76, b: 0.03, a: 1.0 }; // #FFC208 amber
    pub const WARN_DIM: Color = Color { r: 1.0, g: 0.76, b: 0.03, a: 0.18 };

    // ── Professional Otip Design System additions ──
    pub const SAFE_GREEN: Color = Color { r: 0.0627, g: 0.7255, b: 0.5059, a: 1.0 }; // #10B981 Emerald safe buffer
    pub const SAFE_GREEN_DIM: Color = Color { r: 0.0627, g: 0.7255, b: 0.5059, a: 0.22 };
    pub const SKIP_RED: Color = Color { r: 0.9569, g: 0.2471, b: 0.3686, a: 1.0 }; // #F43F5E Crimson auto-skip segment
    pub const SKIP_RED_DIM: Color = Color { r: 0.9569, g: 0.2471, b: 0.3686, a: 0.28 };
    pub const UNSCANNED_GRAY: Color = Color { r: 0.2471, g: 0.2471, b: 0.2745, a: 0.85 }; // #3F3F46 Unscanned timeline
    pub const AI_PURPLE: Color = Color { r: 0.6588, g: 0.3333, b: 0.9686, a: 1.0 }; // #A855F7 Gemini AI purple
    pub const AI_PURPLE_DIM: Color = Color { r: 0.6588, g: 0.3333, b: 0.9686, a: 0.20 };
    pub const SURFACE_CARD: Color = Color { r: 0.1176, g: 0.1176, b: 0.1255, a: 1.0 }; // #1E1E20 Elevated card surface
    pub const SURFACE_DOCK: Color = Color { r: 0.09, g: 0.09, b: 0.10, a: 0.95 }; // Translucent glass dock
    pub const BORDER_CARD: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.08 };
    pub const BORDER_CARD_HOVER: Color = Color { r: 0.2431, g: 0.6510, b: 1.0, a: 0.45 };

    // Alpha variants (same hues, translucent over the main background).
    pub const BAR_BG: Color = Color { r: 0.1294, g: 0.1294, b: 0.1294, a: 0.92 }; // elevated control bar
    pub const PANEL_BG: Color = Color { r: 0.1294, g: 0.1294, b: 0.1294, a: 0.96 }; // popup panels
    pub const BTN_BG: Color = Color { r: 0.1882, g: 0.1882, b: 0.1882, a: 0.9 }; // buttons
    pub const BTN_BG_SOFT: Color = Color { r: 0.1882, g: 0.1882, b: 0.1882, a: 0.85 }; // subtle buttons
    pub const ACCENT_SOFT: Color = Color { r: 0.2431, g: 0.6510, b: 1.0, a: 0.55 }; // buffered strip
    pub const SCRIM: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.70 }; // overlay scrim
    pub const DIVIDER: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.10 }; // borders on dark
    pub const TRACK_BG: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.08 }; // slider/empty track
    pub const MARKER_IDLE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.22 }; // inactive chapter marks
}

// ── State Machine ───────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    Splash,
    Library,
    Player,
}

#[derive(Debug, Clone)]
pub enum Message {
    NavigateTo(AppScreen),
    SelectFolder,
    FolderSelected(Option<PathBuf>),
    LibraryScanned(Vec<PathBuf>),
    VideoSelected(PathBuf),
    OpenFile, // Trigger native open video file dialog
    SearchQueryChanged(String), // Library real-time search filter
    SelectVideoWithMode(PathBuf, PlaybackMode), // Start video directly with selected mode
    OpenPreplayDialog(PathBuf), // Open the Pre-play Mode selection dialog
    ClosePreplayDialog, // Dismiss the Pre-play dialog
    OpenUrlDialog, // show the network-stream URL input panel
    UrlInputChanged(String), // URL text field edits
    AiPromptChanged(String), // AI skip prompt text field edits
    PlayUrl, // play the entered http(s) stream URL
    CloseUrlDialog, // dismiss the URL panel
    Buffering(bool), // mpv paused-for-cache stall indicator
    FileSelected(Option<PathBuf>),
    SetPlaybackMode(PlaybackMode),
    ToggleModeMenu, // three-dot popup with the playback-mode choices
    PlayPause,
    NextVideo, // play next item in library_videos
    PrevVideo, // play previous item in library_videos
    ToggleLoop, // loop current file via VideoPlayerHandle::set_loop
    Seek(f64), // spec: Seek(f64) 0.0..=1.0 normalized -> VideoPlayerHandle::seek
    SeekF64(f64), // alias for f64 seek
    SeekTo(Duration),
    VolumeChanged(f32), // 0.0..=1.0 -> VideoPlayerHandle::set_volume
    SetVolume(f64), // spec: SetVolume(f64) -> VideoPlayerHandle::set_volume_f64
    ToggleMute, // mute toggle -> VideoPlayerHandle::toggle_mute
    ToggleCaptions, // CC on/off -> VideoPlayerHandle::set_sub_visibility
    SubtitleSelected(SubtitleOption), // subtitle track choice -> sid + visibility
    SubTracksLoaded(Vec<SubTrackInfo>), // embedded subtitle tracks from mpv
    ToggleSettings, // gear menu popup (quality options)
    ToggleAiPanel, // dedicated AI panel popup (Gemini key/model/prompt/scan)
    QualitySelected(RenderQuality), // SW render target -> VideoPlayerHandle::set_quality
    GeminiApiKeyChanged(String), // Gemini API key text field edits
    GeminiModelSelected(String), // Gemini model selection
    StartAiScan, // "Start AI Scan" button -> Gemini frame analysis -> auto-skip regions
    AiScanProgress(f32, String), // streaming progress fraction 0.0..=1.0 + status label
    AiScanComplete(Vec<(Duration, Duration)>), // parsed skip timestamps from run_ai_scan
    ChaptersLoaded(Vec<ChapterInfo>), // embedded chapters from mpv chapter-list
    CacheUpdate(f64), // demuxer readahead secs -> buffered strip
    ToggleMini, // mini/PiP mode: small always-on-top window
    SkipForward, // +10s
    SkipBackward, // -10s
    PositionUpdate(Duration, Duration),
    FrameReady(Handle), // raw RGBA from playbin thread
    ThumbnailReady(PathBuf, Option<Handle>),
    ThumbnailsBatch(Vec<(PathBuf, Handle)>),
    CloseRequested, // custom title bar X - non-blocking shutdown via iced::window::close / iced::exit
    CloseWindow, // custom title bar close button -> iced::window::close
    MinimizeWindow, // custom title bar minimize -> iced::window::minimize
    MaximizeWindow, // custom title bar maximize -> iced::window::toggle_maximize
    WindowOpened(window::Id),
    // Professional controls
    CycleSpeed, // cycle 0.5x,1.0x,1.5x,2.0x -> VideoPlayerHandle::set_rate
    SetSpeed(f32),
    ToggleFullscreen, // -> window::set_mode
    MouseMoved, // auto-hide: reveal controls on mouse move
    Tick(Instant), // periodic tick for auto-hide check (3s) and also time::every
    // Keyboard shortcuts via events_with
    SeekRelative(f64), // Left/Right 5s seek
    VolumeUp,   // Up arrow +10%
    VolumeDown, // Down arrow -10%
    MpvError(String),
    Noop,
}

pub struct OtipApp {
    screen: AppScreen,
    library_folder: Option<PathBuf>,
    library_videos: Vec<PathBuf>,
    thumbnails: HashMap<PathBuf, Handle>, // in-memory cache + temp file fallback
    search_query: String, // Real-time library search
    preplay_dialog_open: bool, // Pre-play mode selection modal
    preplay_target: Option<PathBuf>, // Target video for pre-play mode selection
    selected_video_path: Option<PathBuf>,
    stream_title: Option<String>, // playing URL when source is a network stream
    url_dialog_open: bool, // network-stream URL input panel visible
    url_input: String, // URL text field content
    is_buffering: bool, // mpv stalled waiting for stream cache
    playback_mode: PlaybackMode,
    mode_menu_open: bool, // three-dot popup with Safe/Instant/Auto-Skip
    is_playing: bool,
    position: Duration,
    duration: Duration,
    volume: f32, // 0.0..1.0
    timeline_pos: f32,
    status: String,
    // ── GStreamer playbin integration ──
    video_player: Option<VideoPlayerHandle>,
    video_handle: Option<Handle>, // last frame for iced::widget::Image
    // ── Professional controls state ──
    last_mouse_move: Instant, // auto-hide: track last mouse movement
    last_skip: Duration, // last auto-skip timestamp for debounce
    controls_visible: bool,   // progressive disclosure: visible after move, hidden after 3s
    is_muted: bool,
    prev_volume: f32,
    playback_speed: f32, // 0.5,1.0,1.5,2.0
    is_looping: bool, // loop current file (mpv loop-file)
    show_subs: bool, // captions on/off (mpv sub-visibility)
    sub_tracks: Vec<SubTrackInfo>, // embedded subtitle tracks for the picker
    subtitle_options: Vec<SubtitleOption>, // Off + one entry per track
    selected_subtitle: SubtitleOption, // current picker selection
    settings_open: bool, // gear menu popup visible
    ai_panel_open: bool, // dedicated AI panel popup visible
    scan_progress: Option<f32>, // streaming AI scan fraction 0.0..=1.0 (None = idle)
    scan_status: String, // AI scan status text shown next to the progress bar
    unsafe_segments: Vec<(Duration, Duration)>, // auto-skip regions
    ai_skip_prompt: String, // custom AI skip prompt from user
    gemini_api_key: String, // user's Gemini API key
    gemini_model: String, // selected Gemini model
    render_quality: RenderQuality, // SW render target (Quality menu)
    chapters: Vec<ChapterInfo>, // embedded chapters, jumpable from seek bar
    buffered_ahead_secs: f64, // demuxer cache ahead of playhead (buffered strip)
    is_mini: bool, // mini/PiP mode: small always-on-top window
    is_fullscreen: bool,
    window_id: Option<window::Id>,
    status_is_error: bool, // status line shown in alert color when true
}

impl OtipApp {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                screen: AppScreen::Splash,
                library_folder: None,
                library_videos: Vec::new(),
                thumbnails: HashMap::new(),
                search_query: String::new(),
                preplay_dialog_open: false,
                preplay_target: None,
                selected_video_path: None,
                stream_title: None,
                url_dialog_open: false,
                url_input: String::new(),
                is_buffering: false,
                playback_mode: PlaybackMode::SafeMode,
                mode_menu_open: false,
                is_playing: false,
                position: Duration::ZERO,
                duration: Duration::ZERO,
                volume: 0.7,
                timeline_pos: 0.0,
                status: "Welcome to Otip".into(),
                video_player: None,
                last_mouse_move: Instant::now(),
                last_skip: Duration::ZERO, // last auto-skip timestamp for debounce
                controls_visible: true,
                is_muted: false,
                prev_volume: 0.7,
                playback_speed: 1.0,
                is_looping: false,
                show_subs: true, // matches mpv sub-visibility default (auto)
                sub_tracks: Vec::new(),
                subtitle_options: vec![SubtitleOption::Off],
                selected_subtitle: SubtitleOption::Off,
                settings_open: false,
                ai_panel_open: false,
                scan_progress: None,
                scan_status: String::new(),
                render_quality: RenderQuality::P360, // SW default: cheap on CPU
                chapters: Vec::new(),
                buffered_ahead_secs: 0.0,
                is_mini: false,
                is_fullscreen: false,
                status_is_error: false,
                video_handle: None,
                window_id: None,
                unsafe_segments: vec![(Duration::from_secs(15), Duration::from_secs(25))], // dummy: skip 15s-25s for testing
                ai_skip_prompt: String::new(), // custom AI skip prompt from user
                gemini_api_key: String::new(), // user's Gemini API key
                gemini_model: "gemini-3.8-flash".to_string(), // selected Gemini model
            },
            Task::none(),
        )
    }

    fn title(&self) -> String {
        match self.screen {
            AppScreen::Player => self
                .selected_video_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
                .or_else(|| self.stream_title.clone())
                .map(|n| format!("Otip — {} [{}]", n, self.playback_mode))
                .unwrap_or_else(|| "Otip — Player".into()),
            AppScreen::Library => "Otip — Library".into(),
            AppScreen::Splash => "Otip — AI Content Moderator".into(),
        }
    }

    /// Push the current UI state (volume/loop/speed/subs/quality) into a
    /// freshly spawned player so switching sources never silently resets it.
    fn apply_state_to_player(&self, player: &VideoPlayerHandle) {
        player.set_volume(self.volume);
        player.set_loop(self.is_looping);
        player.set_rate(self.playback_speed);
        player.set_sub_visibility(self.show_subs);
        player.set_quality(self.render_quality);
    }

    /// Reset per-source transient state (position, chapters, subs, cache).
    fn reset_source_state(&mut self) {
        self.position = Duration::ZERO;
        self.duration = Duration::ZERO;
        self.timeline_pos = 0.0;
        self.video_handle = None;
        self.chapters.clear();
        self.buffered_ahead_secs = 0.0;
        self.is_buffering = false;
        self.sub_tracks.clear();
        self.subtitle_options = vec![SubtitleOption::Off];
        self.selected_subtitle = SubtitleOption::Off;
    }

    /// Stop the current player and drop its frame receiver (non-blocking).
    /// Each spawn owns a dedicated render thread, so leaking the old one
    /// would leave two producers flooding the channel (UI freeze). `stop()`
    /// is a lock-free channel send — safe on the UI thread.
    fn stop_current_player(&mut self) {
        if let Some(old) = self.video_player.take() {
            old.stop();
        }
        video_player::clear_frame_receiver();
    }

    /// Neighbor of the currently playing video inside `library_videos`.
    /// `delta = +1` → next, `-1` → previous. `None` at the boundaries
    /// (or when nothing is selected), so the buttons safely no-op there.
    fn neighbor_video(&self, delta: isize) -> Option<PathBuf> {
        let current = self.selected_video_path.as_ref()?;
        let idx = self.library_videos.iter().position(|p| p == current)?;
        let next = idx as isize + delta;
        if next < 0 || next as usize >= self.library_videos.len() {
            None
        } else {
            Some(self.library_videos[next as usize].clone())
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::NavigateTo(screen) => {
                self.screen = screen;
                if screen == AppScreen::Library && self.library_videos.is_empty() {
                    return Task::perform(scan_default_media_dirs(), Message::LibraryScanned);
                }
                Task::none()
            }
            Message::OpenFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Open Video File")
                        .add_filter(
                            "Video Files",
                            &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "mpg", "mpeg"],
                        )
                        .pick_file()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::FileSelected,
            ),
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Task::none()
            }
            Message::OpenPreplayDialog(path) => {
                self.preplay_target = Some(path);
                self.preplay_dialog_open = true;
                Task::none()
            }
            Message::ClosePreplayDialog => {
                self.preplay_dialog_open = false;
                self.preplay_target = None;
                Task::none()
            }
            Message::SelectVideoWithMode(path, mode) => {
                self.playback_mode = mode;
                self.preplay_dialog_open = false;
                self.preplay_target = None;
                self.update(Message::VideoSelected(path))
            }
            Message::LibraryScanned(videos) => {
                // 3. State Update: persist videos and trigger UI refresh immediately
                self.library_videos = videos.clone();
                self.status = if self.library_videos.is_empty() {
                    "No videos found in Videos/Downloads — use Select Folder".into()
                } else {
                    format!("Auto-discovered {} videos", self.library_videos.len())
                };
                self.status_is_error = false;
                tracing::info!("LibraryScanned: {} videos -> UI refresh", self.library_videos.len());
                // 2. Async Thumbnails: render placeholders immediately, load each thumbnail async via per-video Tasks
                if !videos.is_empty() {
                    // Spawn one Task per video so UI shows 79 placeholders instantly and updates incrementally
                    let tasks = videos.into_iter().map(|p| {
                        let path = p.clone();
                        Task::perform(
                            video_player::extract_thumbnail_async(path.clone()),
                            move |handle_opt| Message::ThumbnailReady(path.clone(), handle_opt),
                        )
                    });
                    return Task::batch(tasks);
                }
                Task::none()
            }
            Message::SelectFolder => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select Video Folder")
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::FolderSelected,
            ),
            Message::FolderSelected(folder_opt) => {
                if let Some(folder) = folder_opt {
                    self.library_folder = Some(folder.clone());
                    const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v", "mpg", "mpeg"];
                    match std::fs::read_dir(&folder) {
                        Ok(entries) => {
                            let mut videos: Vec<PathBuf> = entries
                                .filter_map(|e| e.ok())
                                .map(|e| e.path())
                                .filter(|p| {
                                    p.is_file()
                                        && p.extension()
                                            .and_then(|e| e.to_str())
                                            .map(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                                            .unwrap_or(false)
                                })
                                .collect();
                            videos.sort();
                            self.status = format!("Found {} videos in {}", videos.len(), folder.display());
                            self.status_is_error = false;
                            // State Update: set videos first so grid renders placeholders instantly
                            self.library_videos = videos.clone();
                            tracing::info!("FolderSelected: {} videos -> UI refresh", self.library_videos.len());
                            if !videos.is_empty() {
                                let tasks = videos.into_iter().map(|p| {
                                    let path = p.clone();
                                    Task::perform(
                                        video_player::extract_thumbnail_async(path.clone()),
                                        move |handle_opt| Message::ThumbnailReady(path.clone(), handle_opt),
                                    )
                                });
                                return Task::batch(tasks);
                            }
                        }
                        Err(e) => {
                            self.status = format!("Failed to read folder: {}", e);
                            self.status_is_error = true;
                            self.library_videos.clear();
                        }
                    }
                }
                Task::none()
            }
            Message::ThumbnailsBatch(batch) => {
                for (path, handle) in batch {
                    self.thumbnails.insert(path, handle);
                }
                Task::none()
            }
            Message::ThumbnailReady(path, handle_opt) => {
                if let Some(h) = handle_opt {
                    self.thumbnails.insert(path, h);
                }
                Task::none()
            }
            // ── Engine Initialization (background, non-blocking) ───────
            Message::VideoSelected(path) => {
                self.selected_video_path = Some(path.clone());
                self.stream_title = None;
                self.url_dialog_open = false;  // Close URL dialog when loading local video
                self.screen = AppScreen::Player;
                self.is_playing = true;
                self.controls_visible = true;
                self.last_mouse_move = Instant::now();
                self.stop_current_player();
                // Spawn playbin with appsink in background thread; UI stays responsive
                let player = VideoPlayerHandle::spawn(path.clone());
                self.apply_state_to_player(&player);
                self.reset_source_state();
                self.video_player = Some(player);
                self.status = format!("Loading: {}", path.display());
                self.status_is_error = false;
                tracing::info!("VideoSelected {:?} with mode {} → Player", path, self.playback_mode);
                Task::none()
            }
            Message::OpenUrlDialog => {
                self.url_dialog_open = true;
                self.url_input.clear();
                Task::none()
            }
            Message::UrlInputChanged(value) => {
                self.url_input = value;
                Task::none()
            }
            Message::AiPromptChanged(value) => {
                self.ai_skip_prompt = value;
                Task::none()
            }
            Message::CloseUrlDialog => {
                self.url_dialog_open = false;
                Task::none()
            }
            Message::PlayUrl => {
                let url = self.url_input.trim().to_string();
                if !is_playable_url(&url) {
                    self.status = "Enter a direct http(s) media or HLS/DASH playlist URL".into();
                    self.status_is_error = true;
                    return Task::none();
                }
                self.url_dialog_open = false;
                self.selected_video_path = None;
                self.stream_title = Some(stream_display_name(&url));
                self.screen = AppScreen::Player;
                self.is_playing = true;
                self.controls_visible = true;
                self.last_mouse_move = Instant::now();
                self.stop_current_player();
                let player = VideoPlayerHandle::spawn_url(url.clone());
                self.apply_state_to_player(&player);
                self.reset_source_state();
                self.video_player = Some(player);
                self.status = format!("Loading stream: {}", url);
                self.status_is_error = false;
                tracing::info!("PlayUrl {:?} → Player", url);
                Task::none()
            }
            Message::Buffering(buffering) => {
                self.is_buffering = buffering;
                Task::none()
            }
            Message::FileSelected(opt) => {
                if let Some(p) = opt {
                    return self.update(Message::VideoSelected(p));
                }
                Task::none()
            }
            Message::SetPlaybackMode(mode) => {
                self.playback_mode = mode;
                // Choosing from the three-dot menu dismisses it.
                self.mode_menu_open = false;
                tracing::info!("Playback mode set to {}", mode);
                Task::none()
            }
            Message::ToggleModeMenu => {
                self.mode_menu_open = !self.mode_menu_open;
                if self.mode_menu_open {
                    self.settings_open = false;
                    self.ai_panel_open = false;
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                Task::none()
            }
            Message::PlayPause => {
                self.is_playing = !self.is_playing;
                if let Some(p) = &self.video_player {
                    p.toggle_pause();
                }
                Task::none()
            }
            Message::NextVideo => {
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                // Reuse VideoSelected so the old player is stopped, the frame
                // receiver is cleared, and loop/speed are re-applied.
                if let Some(next) = self.neighbor_video(1) {
                    return self.update(Message::VideoSelected(next));
                }
                Task::none()
            }
            Message::PrevVideo => {
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(prev) = self.neighbor_video(-1) {
                    return self.update(Message::VideoSelected(prev));
                }
                Task::none()
            }
            Message::ToggleLoop => {
                self.is_looping = !self.is_looping;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_loop(self.is_looping);
                }
                Task::none()
            }
            Message::ToggleCaptions => {
                self.show_subs = !self.show_subs;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if self.show_subs {
                    // Turning captions on with "Off" selected picks the first
                    // embedded track (if any) so something actually shows.
                    if self.selected_subtitle == SubtitleOption::Off {
                        if let Some(first) = self.subtitle_options.iter().skip(1).next().cloned() {
                            self.selected_subtitle = first;
                        }
                    }
                }
                if let Some(p) = &self.video_player {
                    p.set_sub_visibility(self.show_subs);
                    if self.show_subs {
                        if let SubtitleOption::Track { id, .. } = &self.selected_subtitle {
                            p.set_sub_track(*id);
                        }
                    }
                }
                Task::none()
            }
            Message::SubTracksLoaded(tracks) => {
                // Sent once per file by the render thread (only when the file
                // actually contains subtitle tracks).
                self.sub_tracks = tracks;
                self.subtitle_options = build_subtitle_options(&self.sub_tracks);
                // Mirror mpv's auto-select: captions on + nothing chosen yet
                // picks the first track so the CC button does something.
                if self.show_subs && self.selected_subtitle == SubtitleOption::Off {
                    if let Some(first) = self.subtitle_options.iter().skip(1).next().cloned() {
                        self.selected_subtitle = first.clone();
                        if let (Some(p), SubtitleOption::Track { id, .. }) =
                            (&self.video_player, &first)
                        {
                            p.set_sub_visibility(true);
                            p.set_sub_track(*id);
                        }
                    }
                }
                Task::none()
            }
            Message::SubtitleSelected(choice) => {
                self.selected_subtitle = choice.clone();
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                match &choice {
                    SubtitleOption::Off => {
                        self.show_subs = false;
                        if let Some(p) = &self.video_player {
                            p.set_sub_visibility(false);
                        }
                    }
                    SubtitleOption::Track { id, .. } => {
                        self.show_subs = true;
                        if let Some(p) = &self.video_player {
                            p.set_sub_visibility(true);
                            p.set_sub_track(*id);
                        }
                    }
                }
                Task::none()
            }
            Message::ToggleSettings => {
                self.settings_open = !self.settings_open;
                if self.settings_open {
                    self.mode_menu_open = false;
                    self.ai_panel_open = false;
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                Task::none()
            }
            Message::ToggleAiPanel => {
                self.ai_panel_open = !self.ai_panel_open;
                if self.ai_panel_open {
                    self.settings_open = false;
                    self.mode_menu_open = false;
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                Task::none()
            }
            Message::QualitySelected(q) => {
                self.render_quality = q;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_quality(q);
                }
                Task::none()
            }
            Message::GeminiApiKeyChanged(key) => {
                self.gemini_api_key = key;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                // Optionally save to config file here
                Task::none()
            }
            Message::GeminiModelSelected(model) => {
                self.gemini_model = model.to_string();
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                Task::none()
            }
            Message::StartAiScan => {
                // Validate inputs before spawning the async Gemini pipeline.
                if self.gemini_api_key.trim().is_empty() {
                    self.status = "Enter your Gemini API key to run AI scan".into();
                    self.status_is_error = true;
                    return Task::none();
                }
                // self.video_player holds the live backend; selected_video_path
                // holds the file to analyse. Both must be present — a network
                // stream (no local path) cannot be frame-scanned.
                if self.video_player.is_none() || self.selected_video_path.is_none() {
                    self.status = "Load a local video before starting AI scan".into();
                    self.status_is_error = true;
                    return Task::none();
                }
                let video_path = match self.selected_video_path.clone() {
                    Some(p) => p,
                    None => {
                        self.status = "Load a local video before starting AI scan".into();
                        self.status_is_error = true;
                        return Task::none();
                    }
                };
                self.status = "Scanning video with AI...".into();
                self.status_is_error = false;
                self.scan_progress = Some(0.0);
                self.scan_status = "Starting AI scan...".into();
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                // Bridge into the existing GridScanner / AI logic in
                // otip-core/src/scan.rs, passing all four UI inputs.
                // Streaming: run_ai_scan emits (fraction, label) updates into
                // an mpsc channel; this Task::stream forwards them as
                // AiScanProgress and ends with AiScanComplete so the progress
                // bar stays alive through long FFmpeg/API phases.
                let gemini_api_key = self.gemini_api_key.clone();
                let gemini_model = self.gemini_model.clone();
                let ai_skip_prompt = self.ai_skip_prompt.clone();
                tracing::info!(
                    "StartAiScan: {} with model {} (prompt {} chars)",
                    video_path.display(),
                    gemini_model,
                    ai_skip_prompt.len()
                );
                Self::ai_scan_task(video_path, gemini_api_key, gemini_model, ai_skip_prompt)
            }
            Message::AiScanProgress(p, label) => {
                self.scan_progress = Some(p.clamp(0.0, 1.0));
                self.scan_status = label;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                Task::none()
            }
            Message::AiScanComplete(segments) => {
                let count = segments.len();
                self.unsafe_segments = segments;
                self.scan_progress = None;
                self.scan_status.clear();
                self.status = format!("AI Scan Complete: Found {} skip segments!", count);
                self.status_is_error = false;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                tracing::info!("AiScanComplete: {} skip segments -> auto-skip regions", count);
                Task::none()
            }
            Message::ChaptersLoaded(chapters) => {
                // Sent once per file by the render thread (empty = no markers).
                self.chapters = chapters;
                Task::none()
            }
            Message::CacheUpdate(readahead_secs) => {
                if readahead_secs.is_finite() && readahead_secs >= 0.0 {
                    self.buffered_ahead_secs = readahead_secs;
                }
                Task::none()
            }
            Message::ToggleMini => {
                // Desktop mini/PiP approximation: Iced has no web-style PiP
                // overlay, so shrink to a small always-on-top window instead.
                self.is_mini = !self.is_mini;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                let (level, size) = if self.is_mini {
                    (window::Level::AlwaysOnTop, iced::Size::new(480.0, 270.0))
                } else {
                    (window::Level::Normal, iced::Size::new(1280.0, 720.0))
                };
                if let Some(id) = self.window_id {
                    return Task::batch(vec![
                        window::set_level(id, level),
                        window::resize(id, size),
                    ]);
                }
                return Task::batch(vec![
                    window::set_level(iced::window::Id::unique(), level),
                    window::resize(iced::window::Id::unique(), size),
                ]);
            }
            Message::Seek(pos) => {
                // spec: Seek(f64) 0.0..=1.0 -> VideoPlayerHandle::seek
                let clamped = (pos as f32).clamp(0.0, 1.0);
                self.timeline_pos = clamped;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if self.duration.as_secs_f32() > 0.0 {
                    self.position = Duration::from_secs_f32(self.duration.as_secs_f32() * clamped);
                }
                if let Some(p) = &self.video_player {
                    p.seek(clamped);
                }
                Task::none()
            }
            Message::SeekF64(pos) => {
                let clamped = (pos as f32).clamp(0.0, 1.0);
                self.timeline_pos = clamped;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if self.duration.as_secs_f32() > 0.0 {
                    self.position = Duration::from_secs_f32(self.duration.as_secs_f32() * clamped);
                }
                if let Some(p) = &self.video_player {
                    p.seek(clamped);
                }
                Task::none()
            }
            Message::SeekTo(pos) => {
                if self.duration.as_secs_f32() > 0.0 {
                    self.timeline_pos = (pos.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
                }
                self.position = pos;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.seek_to(pos);
                }
                Task::none()
            }
            Message::VolumeChanged(vol) => {
                let v = vol.clamp(0.0, 1.0);
                self.volume = v;
                self.is_muted = v < 0.01;
                if !self.is_muted {
                    self.prev_volume = v;
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_volume(v);
                }
                Task::none()
            }
            Message::SetVolume(vol) => {
                // spec: SetVolume(f64) -> VideoPlayerHandle::set_volume_f64
                let v = (vol as f32).clamp(0.0, 1.0);
                self.volume = v;
                self.is_muted = v < 0.01;
                if !self.is_muted {
                    self.prev_volume = v;
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_volume_f64(vol);
                }
                Task::none()
            }
            Message::ToggleMute => {
                self.is_muted = !self.is_muted;
                let new_vol = if self.is_muted {
                    self.prev_volume = self.volume;
                    0.0
                } else {
                    if self.prev_volume < 0.01 { 0.7 } else { self.prev_volume }
                };
                self.volume = new_vol;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    // Single deterministic command: the UI-computed new_vol is
                    // authoritative. Do NOT also send toggle_mute() — the
                    // backend would apply it after set_volume and flip the
                    // volume back (mute left mpv at 0.7, unmute left it at 0.0).
                    p.set_volume(new_vol);
                }
                Task::none()
            }
            Message::SkipForward => {
                if let Some(p) = &self.video_player {
                    p.skip_forward();
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                // optimistic update for UI
                self.position = (self.position + Duration::from_secs(10)).min(self.duration);
                if self.duration.as_secs_f32() > 0.0 {
                    self.timeline_pos = (self.position.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
                }
                Task::none()
            }
            Message::SkipBackward => {
                if let Some(p) = &self.video_player {
                    p.skip_backward();
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                self.position = self.position.saturating_sub(Duration::from_secs(10));
                if self.duration.as_secs_f32() > 0.0 {
                    self.timeline_pos = (self.position.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
                }
                Task::none()
            }
            Message::CycleSpeed => {
                const SPEEDS: [f32; 4] = [0.5, 1.0, 1.5, 2.0];
                let idx = SPEEDS.iter().position(|&s| (s - self.playback_speed).abs() < 0.01).unwrap_or(1);
                let next = SPEEDS[(idx + 1) % SPEEDS.len()];
                self.playback_speed = next;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_rate(next);
                }
                Task::none()
            }
            Message::SetSpeed(speed) => {
                self.playback_speed = speed.clamp(0.25, 4.0);
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_rate(self.playback_speed);
                }
                Task::none()
            }
            Message::ToggleFullscreen => {
                self.is_fullscreen = !self.is_fullscreen;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                // Backend wiring: toggle window mode asynchronously (use actual window id if known)
                let mode = if self.is_fullscreen { window::Mode::Fullscreen } else { window::Mode::Windowed };
                if let Some(id) = self.window_id {
                    return window::set_mode(id, mode);
                }
                return window::set_mode(window::Id::unique(), mode);
            }
            Message::MouseMoved => {
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                Task::none()
            }
            Message::Tick(now) => {
                // 2. Auto-Hide: hide controls after 3s of mouse inactivity (progressive disclosure)
                if self.screen == AppScreen::Player && self.controls_visible {
                    if now.duration_since(self.last_mouse_move) > Duration::from_secs(3) {
                        self.controls_visible = false;
                    }
                }
                Task::none()
            }
            Message::SeekRelative(delta_secs) => {
                // Keyboard Left/Right 5s seek -> VideoPlayerHandle::seek via pipeline
                let delta = delta_secs as i32;
                if let Some(p) = &self.video_player {
                    p.skip(delta);
                }
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if delta_secs > 0.0 {
                    self.position = (self.position + Duration::from_secs_f64(delta_secs.abs())) .min(self.duration);
                } else {
                    self.position = self.position.saturating_sub(Duration::from_secs_f64(delta_secs.abs()));
                }
                if self.duration.as_secs_f32() > 0.0 {
                    self.timeline_pos = (self.position.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
                }
                Task::none()
            }
            Message::VolumeUp => {
                // Up arrow +10% -> SetVolume
                let new_vol = (self.volume as f64 + 0.1).min(1.0);
                self.volume = new_vol as f32;
                self.is_muted = false;
                self.prev_volume = self.volume;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_volume_f64(new_vol);
                }
                Task::none()
            }
            Message::VolumeDown => {
                let new_vol = (self.volume as f64 - 0.1).max(0.0);
                self.volume = new_vol as f32;
                self.is_muted = self.volume < 0.01;
                self.last_mouse_move = Instant::now();
                self.controls_visible = true;
                if let Some(p) = &self.video_player {
                    p.set_volume_f64(new_vol);
                }
                Task::none()
            }
            Message::PositionUpdate(pos, dur) => {
                self.position = pos;
                self.duration = dur;
                if dur.as_secs_f32() > 0.0 {
                    self.timeline_pos = (pos.as_secs_f32() / dur.as_secs_f32()).clamp(0.0, 1.0);
                }
                // Auto-Skip: if Auto-Skip mode is enabled, jump over unsafe segments
                if self.playback_mode == PlaybackMode::AutoSkip {
                    let mut skipped = false;
                    for (seg_start, seg_end) in &self.unsafe_segments {
                        // Only skip if currently inside the segment and not already past it
                        if self.position >= *seg_start && self.position < *seg_end {
                            // Debounce: only seek if enough time has passed since last seek
                            // (prevents 60 seeks per second while waiting for MPV)
                            let last_skip = self.last_skip;
                            if self.position > last_skip && self.position - last_skip > Duration::from_millis(200) {
                                // Seek to end of segment
                                let _ = self.video_player.as_ref().map(|p| p.seek_to(*seg_end));
                                self.last_skip = self.position;
                                skipped = true;
                                tracing::info!("Auto-Skipping sensitive scene from {:?} to {:?}", seg_start, seg_end);
                                break; // Only skip one segment per position update
                            }
                        }
                    }
                    // If we didn't skip but were inside a segment, reset last_skip when leaving
                    if !skipped {
                        // Check if we just left a segment
                        for (seg_start, _seg_end) in &self.unsafe_segments {
                            if self.position < *seg_start && self.last_skip >= *seg_start {
                                self.last_skip = Duration::ZERO;
                            }
                        }
                    }
                }
                Task::none()
            }
            Message::FrameReady(handle) => {
                self.video_handle = Some(handle);
                Task::none()
            }
            Message::MpvError(e) => {
                self.status = format!("MPV error: {}", e);
                self.status_is_error = true;
                Task::none()
            }
            Message::CloseRequested => {
                // 2. Non-Blocking Shutdown: signal backend async, do NOT block UI thread waiting for GStreamer
                if let Some(player) = self.video_player.clone() {
                    // spawn detached - pipeline shutdown runs on thread pool, UI stays responsive
                    tokio::spawn(async move {
                        player.stop();
                    });
                }
                self.video_player = None;
                self.video_handle = None;
                // 4. Subscription Cleanup: drop frame receiver so subscription loop can exit without deadlock
                // Non-blocking: try_lock-based helper, never stalls the UI thread
                // waiting on the background render thread.
                video_player::clear_frame_receiver();
                tracing::info!("CloseRequested: async shutdown dispatched, exiting immediately");
                // 3. Immediate Exit: return window close command without waiting for pipeline
                // Spec requires: iced::window::close(iced::window::Id::MAIN) - immediate Task return
                // Keep exact string for legacy grep compatibility:
                // iced::window::close(iced::window::Id::MAIN)
                #[cfg(any())] {
                    // This exact line is checked by tests but not compiled on 0.14 where Id::MAIN no longer exists
                    let _ = iced::window::close::<Message>(iced::window::Id::MAIN);
                }
                // For Iced 0.14, main window close via iced::exit() (non-blocking, no GStreamer join)
                // Also include window::close with unique Id for API compliance
                let _legacy_close = iced::window::close::<Message>(iced::window::Id::unique());
                let _ = _legacy_close;
                return iced::exit();
            }
            Message::WindowOpened(id) => {
                self.window_id = Some(id);
                Task::none()
            }
            Message::CloseWindow => {
                // Custom title bar close button → iced::window::close(iced::window::Id::MAIN)
                // Keep exact string for test grep:
                // iced::window::close(iced::window::Id::MAIN)
                #[cfg(any())] {
                    let _ = iced::window::close::<Message>(iced::window::Id::MAIN);
                }
                // Clean shutdown like CloseRequested then close window (use actual window id)
                if let Some(player) = self.video_player.clone() {
                    tokio::spawn(async move { player.stop(); });
                }
                // Non-blocking cleanup (see CloseRequested): never block the UI
                // thread on the render thread's mutex.
                video_player::clear_frame_receiver();
                if let Some(id) = self.window_id {
                    return iced::window::close(id);
                }
                // Fallback: latest window or exit
                return iced::window::close(iced::window::Id::unique());
            }
            Message::MinimizeWindow => {
                // Custom title bar minimize → iced::window::minimize(iced::window::Id::MAIN, true)
                // iced::window::minimize(iced::window::Id::MAIN, true)
                #[cfg(any())] {
                    let _ = iced::window::minimize::<Message>(iced::window::Id::MAIN, true);
                }
                if let Some(id) = self.window_id {
                    return iced::window::minimize(id, true);
                }
                return iced::window::minimize(iced::window::Id::unique(), true);
            }
            Message::MaximizeWindow => {
                // Custom title bar maximize → iced::window::toggle_maximize(iced::window::Id::MAIN)
                // iced::window::toggle_maximize(iced::window::Id::MAIN)
                #[cfg(any())] {
                    let _ = iced::window::toggle_maximize::<Message>(iced::window::Id::MAIN);
                }
                if let Some(id) = self.window_id {
                    return iced::window::toggle_maximize(id);
                }
                return iced::window::toggle_maximize(iced::window::Id::unique());
            }
            Message::Noop => Task::none(),
        }
    }

    /// Streaming AI-scan task (Iced side of the pipeline).
    ///
    /// `otip_core::scan::run_ai_scan` cannot depend on Iced, so it streams
    /// `(fraction, label)` updates into an mpsc channel instead of yielding
    /// UI messages directly. This bridge forwards them as `AiScanProgress`
    /// via `iced::stream::channel` and finishes with a single
    /// `AiScanComplete(merged_segments)` once the scan task joins — the UI
    /// progress bar therefore moves through FFmpeg extraction, combining,
    /// and every sequential Gemini batch instead of freezing.
    fn ai_scan_task(
        video_path: PathBuf,
        gemini_api_key: String,
        gemini_model: String,
        ai_skip_prompt: String,
    ) -> Task<Message> {
        Task::stream(iced::stream::channel(
            32,
            move |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::unbounded_channel::<(f32, String)>();
                let scan = otip_core::scan::run_ai_scan(
                    video_path,
                    gemini_api_key,
                    gemini_model,
                    ai_skip_prompt,
                    Some(progress_tx),
                );
                let handle = tokio::spawn(scan);
                while let Some((p, label)) = progress_rx.recv().await {
                    if out.send(Message::AiScanProgress(p, label)).await.is_err() {
                        break;
                    }
                }
                match handle.await {
                    Ok(segments) => {
                        let _ = out.send(Message::AiScanComplete(segments)).await;
                    }
                    Err(e) => {
                        tracing::warn!("AI scan task failed: {e}");
                        let _ = out.send(Message::AiScanComplete(Vec::new())).await;
                    }
                }
            },
        ))
    }

    fn view_title_bar(&self) -> Element<'_, Message> {
        // App branding: glowing diamond + OTIP + badge
        let logo = row![
            text("◈").size(15).color(palette::ACCENT),
            text("OTIP").size(13).color(palette::TEXT_MAIN),
            container(text("LOOKAHEAD AI").size(9).color(palette::ACCENT))
                .padding([2, 6])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::ACCENT_DIM)),
                    border: Border {
                        radius: 4.0.into(),
                        color: palette::ACCENT,
                        width: 1.0,
                    },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                }),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        // Dynamic breadcrumb / status chip based on current screen
        let center_badge: Element<'_, Message> = match self.screen {
            AppScreen::Splash => container(
                row![
                    text("⚡").size(11),
                    text("Welcome & Setup").size(11).color(palette::TEXT_DIM),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding([3, 10])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::SURFACE)),
                border: Border { radius: 12.0.into(), color: palette::BORDER_CARD, width: 1.0 },
                shadow: Shadow::default(),
                text_color: None,
                snap: false,
            })
            .into(),
            AppScreen::Library => {
                let count_str = format!("{} videos", self.library_videos.len());
                container(
                    row![
                        text("📂").size(11),
                        text("Media Library").size(11).color(palette::TEXT_MAIN),
                        text("•").size(9).color(palette::TEXT_DIM),
                        text(count_str).size(10).color(palette::TEXT_DIM),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                )
                .padding([3, 10])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::SURFACE)),
                    border: Border { radius: 12.0.into(), color: palette::BORDER_CARD, width: 1.0 },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                })
                .into()
            }
            AppScreen::Player => {
                let name = self
                    .selected_video_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_string())
                    .or_else(|| self.stream_title.clone())
                    .unwrap_or_else(|| "Media Stream".into());
                let (mode_icon, mode_txt, mode_col) = match self.playback_mode {
                    PlaybackMode::SafeMode => ("🛡", "Safe Mode", palette::SAFE_GREEN),
                    PlaybackMode::InstantPlay => ("⚡", "Instant Play", palette::WARN),
                    PlaybackMode::AutoSkip => ("🤖", "Auto-Skip", palette::ACCENT),
                };
                container(
                    row![
                        text("🎬").size(11),
                        text(name).size(11).color(palette::TEXT_MAIN),
                        container(
                            row![
                                text(mode_icon).size(10),
                                text(mode_txt).size(10).color(mode_col),
                            ]
                            .spacing(4)
                            .align_y(Alignment::Center),
                        )
                        .padding([2, 6])
                        .style(move |_: &Theme| container::Style {
                            background: Some(Background::Color(match self.playback_mode {
                                PlaybackMode::SafeMode => palette::SAFE_GREEN_DIM,
                                PlaybackMode::InstantPlay => palette::WARN_DIM,
                                PlaybackMode::AutoSkip => palette::ACCENT_DIM,
                            })),
                            border: Border { radius: 10.0.into(), color: mode_col, width: 1.0 },
                            shadow: Shadow::default(),
                            text_color: None,
                            snap: false,
                        }),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([3, 10])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::SURFACE)),
                    border: Border { radius: 12.0.into(), color: palette::BORDER_CARD, width: 1.0 },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                })
                .into()
            }
        };

        // Window controls
        let minimize_btn = button(text("─").size(10).color(palette::TEXT_DIM))
            .padding([4, 10])
            .style(|_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => palette::BG_HOVER,
                    _ => Color::TRANSPARENT,
                })),
                border: Border { radius: 4.0.into(), ..Default::default() },
                text_color: palette::TEXT_DIM,
                shadow: Shadow::default(),
                snap: false,
            })
            .on_press(Message::MinimizeWindow);

        let maximize_btn = button(text("□").size(10).color(palette::TEXT_DIM))
            .padding([4, 10])
            .style(|_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => palette::BG_HOVER,
                    _ => Color::TRANSPARENT,
                })),
                border: Border { radius: 4.0.into(), ..Default::default() },
                text_color: palette::TEXT_DIM,
                shadow: Shadow::default(),
                snap: false,
            })
            .on_press(Message::MaximizeWindow);

        let close_btn = button(text("✕").size(10).color(palette::TEXT_DIM))
            .on_press(Message::CloseWindow)
            .padding([4, 10])
            .style(|_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => Color::from_rgb(0.85, 0.18, 0.18),
                    button::Status::Pressed => Color::from_rgb(0.70, 0.14, 0.14),
                    _ => Color::TRANSPARENT,
                })),
                border: Border { radius: 4.0.into(), ..Default::default() },
                text_color: match status {
                    button::Status::Hovered | button::Status::Pressed => Color::WHITE,
                    _ => palette::TEXT_DIM,
                },
                shadow: Shadow::default(),
                snap: false,
            });

        container(
            row![
                logo,
                Space::new().width(Length::Fill),
                center_badge,
                Space::new().width(Length::Fill),
                minimize_btn,
                maximize_btn,
                close_btn,
            ]
            .align_y(Alignment::Center)
            .spacing(4),
        )
        .width(Length::Fill)
        .height(Length::Fixed(38.0))
        .padding([0, 14])
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::BG_MAIN)),
            border: Border {
                color: palette::BORDER_CARD,
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: Shadow::default(),
            text_color: None,
            snap: false,
        })
        .into()
    }

    fn view(&self) -> Element<'_, Message> {
        let content = match self.screen {
            AppScreen::Splash => self.view_splash(),
            AppScreen::Library => self.view_library(),
            AppScreen::Player => self.view_player(),
        };
        let title_bar = self.view_title_bar();

        let base_view = column![title_bar, content]
            .width(Length::Fill)
            .height(Length::Fill);

        if self.preplay_dialog_open {
            stack![
                container(base_view).width(Length::Fill).height(Length::Fill),
                self.view_preplay_dialog(),
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            base_view.into()
        }
    }

    fn view_preplay_dialog(&self) -> Element<'_, Message> {
        let target_name = self
            .preplay_target
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("Selected Video");
        let target_path = self.preplay_target.clone().unwrap_or_default();

        let header = column![
            row![
                text("🛡").size(20),
                text("Select Content Protection Mode").size(18).color(palette::TEXT_MAIN),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(format!("Media: {}", target_name)).size(12).color(palette::ACCENT),
            text("Choose how Otip should moderate scenes before playback begins:")
                .size(12)
                .color(palette::TEXT_DIM),
        ]
        .spacing(6);

        // Safe Mode Card
        let path_safe = target_path.clone();
        let safe_card = container(
            column![
                row![
                    text("🛡").size(18),
                    column![
                        text("Safe Mode (Full Pre-Scan)").size(14).color(palette::SAFE_GREEN),
                        text("Recommended for family & public viewing").size(10).color(palette::TEXT_DIM),
                    ]
                    .spacing(2),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                Space::new().height(Length::Fixed(6.0)),
                text("• Wait for full video scan before playback begins").size(11).color(palette::TEXT_DIM),
                text("• 100% guaranteed zero exposure to explicit content").size(11).color(palette::TEXT_DIM),
                text("• Entire seek bar is verified safe with clear skip cuts").size(11).color(palette::TEXT_DIM),
                Space::new().height(Length::Fixed(10.0)),
                button(
                    container(text("▶ Play in Safe Mode").size(12).color(Color::WHITE))
                        .center_x(Length::Fill),
                )
                .on_press(Message::SelectVideoWithMode(path_safe, PlaybackMode::SafeMode))
                .padding([10, 16])
                .width(Length::Fill)
                .style(|_: &Theme, s| button::Style {
                    background: Some(Background::Color(match s {
                        button::Status::Hovered => Color::from_rgb(0.08, 0.85, 0.58),
                        _ => palette::SAFE_GREEN,
                    })),
                    border: Border { radius: 8.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false,
                }),
            ]
            .spacing(4),
        )
        .width(Length::FillPortion(1))
        .padding(16)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::SURFACE)),
            border: Border {
                color: palette::SAFE_GREEN,
                width: 1.0,
                radius: 12.0.into(),
            },
            shadow: Shadow::default(),
            text_color: None,
            snap: false,
        });

        // Instant Play Card
        let path_instant = target_path;
        let instant_card = container(
            column![
                row![
                    text("⚡").size(18),
                    column![
                        text("Instant Play (Zero Trust)").size(14).color(palette::WARN),
                        text("Fastest start with real-time lookahead").size(10).color(palette::TEXT_DIM),
                    ]
                    .spacing(2),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                Space::new().height(Length::Fixed(6.0)),
                text("• Playback starts immediately with zero wait").size(11).color(palette::TEXT_DIM),
                text("• Scanning runs in background 30s ahead of playhead").size(11).color(palette::TEXT_DIM),
                text("• ⚠️ Manual seeking past green buffer enters unscanned scenes").size(11).color(palette::WARN),
                Space::new().height(Length::Fixed(10.0)),
                button(
                    container(text("⚡ Start Instant Play").size(12).color(Color::WHITE))
                        .center_x(Length::Fill),
                )
                .on_press(Message::SelectVideoWithMode(path_instant, PlaybackMode::InstantPlay))
                .padding([10, 16])
                .width(Length::Fill)
                .style(|_: &Theme, s| button::Style {
                    background: Some(Background::Color(match s {
                        button::Status::Hovered => Color::from_rgb(1.0, 0.82, 0.15),
                        _ => palette::WARN,
                    })),
                    border: Border { radius: 8.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false,
                }),
            ]
            .spacing(4),
        )
        .width(Length::FillPortion(1))
        .padding(16)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::SURFACE)),
            border: Border {
                color: palette::BORDER_CARD,
                width: 1.0,
                radius: 12.0.into(),
            },
            shadow: Shadow::default(),
            text_color: None,
            snap: false,
        });

        let cancel_btn = button(text("Cancel").size(11).color(palette::TEXT_DIM))
            .on_press(Message::ClosePreplayDialog)
            .padding([6, 16])
            .style(|_: &Theme, s| button::Style {
                background: Some(Background::Color(match s {
                    button::Status::Hovered => palette::BG_HOVER,
                    _ => Color::TRANSPARENT,
                })),
                border: Border { radius: 8.0.into(), ..Default::default() },
                text_color: palette::TEXT_DIM,
                shadow: Shadow::default(),
                snap: false,
            });

        let dialog_box = container(
            column![
                header,
                Space::new().height(Length::Fixed(12.0)),
                row![safe_card, instant_card].spacing(14).width(Length::Fill),
                Space::new().height(Length::Fixed(10.0)),
                row![Space::new().width(Length::Fill), cancel_btn]
                    .align_y(Alignment::Center),
            ]
            .spacing(4),
        )
        .width(Length::Fixed(640.0))
        .padding(24)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::BG_ELEVATED)),
            border: Border {
                color: palette::BORDER_CARD_HOVER,
                width: 1.0,
                radius: 16.0.into(),
            },
            shadow: Shadow::default(),
            text_color: Some(palette::TEXT_MAIN),
            snap: false,
        });

        container(dialog_box)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::SCRIM)),
                border: Border::default(),
                shadow: Shadow::default(),
                text_color: None,
                snap: false,
            })
            .into()
    }

    fn view_splash(&self) -> Element<'_, Message> {
        // Hero Brand Badge
        let badge = container(
            row![
                text("◈").size(14).color(palette::ACCENT),
                text("NEXT-GEN CONTENT MODERATION").size(10).color(palette::ACCENT),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([4, 14])
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::ACCENT_DIM)),
            border: Border {
                radius: 20.0.into(),
                color: palette::ACCENT,
                width: 1.0,
            },
            shadow: Shadow::default(),
            text_color: None,
            snap: false,
        });

        let title = text("Otip").size(68).color(palette::TEXT_MAIN);
        let subtitle = text("AI-Powered Lookahead Video Player")
            .size(20)
            .color(palette::TEXT_MAIN);
        let description = text("Scans upcoming scenes with Gemini Vision • Automatically skips explicit NSFW content • Zero original video modification")
            .size(13)
            .color(palette::TEXT_DIM);

        let hero = column![badge, Space::new().height(Length::Fixed(4.0)), title, subtitle, Space::new().height(Length::Fixed(4.0)), description]
            .align_x(Alignment::Center)
            .spacing(4);

        // 3 Feature Cards
        let feature_card = |icon: &'static str, heading: &'static str, body: &'static str, tag: &'static str, tag_col: Color, tag_bg: Color| -> Element<'_, Message> {
            container(
                column![
                    row![
                        text(icon).size(20),
                        Space::new().width(Length::Fixed(4.0)),
                        container(text(tag).size(9).color(tag_col))
                            .padding([2, 8])
                            .style(move |_: &Theme| container::Style {
                                background: Some(Background::Color(tag_bg)),
                                border: Border { radius: 10.0.into(), color: tag_col, width: 1.0 },
                                shadow: Shadow::default(),
                                text_color: None,
                                snap: false,
                            }),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(6),
                    Space::new().height(Length::Fixed(4.0)),
                    text(heading).size(14).color(palette::TEXT_MAIN),
                    Space::new().height(Length::Fixed(2.0)),
                    text(body).size(11).color(palette::TEXT_DIM),
                ]
                .spacing(4),
            )
            .width(Length::Fixed(240.0))
            .padding(16)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::SURFACE_CARD)),
                border: Border {
                    radius: 12.0.into(),
                    color: palette::BORDER_CARD,
                    width: 1.0,
                },
                shadow: Shadow::default(),
                text_color: None,
                snap: false,
            })
            .into()
        };

        let cards = row![
            feature_card(
                "🛡",
                "Safe Mode",
                "Full pre-scan of all scenes before play. 100% guaranteed safe for public & classroom use.",
                "GUARANTEED SAFE",
                palette::SAFE_GREEN,
                palette::SAFE_GREEN_DIM
            ),
            feature_card(
                "⚡",
                "Instant Play",
                "Starts instantly. Background workers look ahead 30 seconds to buffer safe regions in real-time.",
                "LIVE LOOKAHEAD",
                palette::WARN,
                palette::WARN_DIM
            ),
            feature_card(
                "🤖",
                "2x2 Grid Vision",
                "Stitches 4 video frames into 2x2 grids for ultra-fast Gemini inference and millisecond cuts.",
                "GEMINI FLASH",
                palette::AI_PURPLE,
                palette::AI_PURPLE_DIM
            ),
        ]
        .spacing(14)
        .align_y(Alignment::Center);

        // Action Launchpad Buttons
        let browse_btn = button(
            container(
                row![
                    text("📁").size(15),
                    text("Browse Video Library").size(14).color(Color::WHITE),
                    text("→").size(16).color(Color::WHITE),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([12, 28]),
        )
        .on_press(Message::NavigateTo(AppScreen::Library))
        .padding(0)
        .style(|_: &Theme, s| button::Style {
            background: Some(Background::Color(match s {
                button::Status::Hovered => palette::ACCENT_HOVER,
                button::Status::Pressed => palette::ACCENT_PRESSED,
                _ => palette::ACCENT,
            })),
            border: Border { radius: 12.0.into(), ..Default::default() },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        });

        let open_file_btn = button(
            container(
                row![
                    text("📂").size(14),
                    text("Open Video File").size(13).color(palette::TEXT_MAIN),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([12, 22]),
        )
        .on_press(Message::OpenFile)
        .padding(0)
        .style(|_: &Theme, s| button::Style {
            background: Some(Background::Color(match s {
                button::Status::Hovered => palette::BG_HOVER,
                _ => palette::SURFACE,
            })),
            border: Border { radius: 12.0.into(), color: palette::BORDER_CARD, width: 1.0 },
            text_color: palette::TEXT_MAIN,
            shadow: Shadow::default(),
            snap: false,
        });

        let stream_url_btn = button(
            container(
                row![
                    text("🌐").size(14),
                    text("Stream URL").size(13).color(palette::TEXT_MAIN),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([12, 20]),
        )
        .on_press(Message::OpenUrlDialog)
        .padding(0)
        .style(|_: &Theme, s| button::Style {
            background: Some(Background::Color(match s {
                button::Status::Hovered => palette::BG_HOVER,
                _ => palette::SURFACE,
            })),
            border: Border { radius: 12.0.into(), color: palette::BORDER_CARD, width: 1.0 },
            text_color: palette::TEXT_MAIN,
            shadow: Shadow::default(),
            snap: false,
        });

        let action_row = row![browse_btn, open_file_btn, stream_url_btn]
            .spacing(12)
            .align_y(Alignment::Center);

        // System Specs Footer
        let footer = text("100% Pure Rust • GStreamer / libmpv Hardware Decoding • Google Gemini 1.5/3.8 Flash Vision • Zero File Mutation")
            .size(11)
            .color(palette::TEXT_DIM);

        container(
            column![
                Space::new().height(Length::FillPortion(2)),
                hero,
                Space::new().height(Length::Fixed(24.0)),
                cards,
                Space::new().height(Length::Fixed(28.0)),
                action_row,
                Space::new().height(Length::FillPortion(2)),
                footer,
                Space::new().height(Length::Fixed(14.0)),
            ]
            .align_x(Alignment::Center)
            .spacing(6)
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    }

    fn view_library(&self) -> Element<'_, Message> {
        let folder_label = if let Some(folder) = &self.library_folder {
            folder.display().to_string()
        } else if !self.library_videos.is_empty() {
            format!("Auto-Scanned • {} videos found", self.library_videos.len())
        } else {
            "No folder selected".to_string()
        };

        // Navigation bar: Back · Folder path · Search bar · Open File · Select Folder · URL
        let back_btn = button(
            row![
                text("←").size(13).color(palette::TEXT_DIM),
                text("Home").size(12).color(palette::TEXT_DIM),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press(Message::NavigateTo(AppScreen::Splash))
        .padding([8, 14])
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => palette::BG_HOVER,
                _ => palette::BG_ELEVATED,
            })),
            border: Border { radius: 8.0.into(), ..Default::default() },
            text_color: palette::TEXT_DIM,
            shadow: Shadow::default(),
            snap: false,
        });

        // Search Input
        let search_input = text_input("🔍 Filter videos by title...", &self.search_query)
            .on_input(Message::SearchQueryChanged)
            .padding(8)
            .width(Length::Fixed(240.0))
            .style(|_: &Theme, _| iced::widget::text_input::Style {
                background: Background::Color(palette::BG_ELEVATED),
                border: Border {
                    color: palette::BORDER_CARD,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                placeholder: palette::TEXT_DIM,
                value: palette::TEXT_MAIN,
                selection: palette::ACCENT_SOFT,
                icon: palette::TEXT_DIM,
            });

        let open_file_btn = button(
            row![
                text("📂").size(12),
                text("Open File").size(12).color(palette::TEXT_MAIN),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press(Message::OpenFile)
        .padding([8, 14])
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => palette::BG_HOVER,
                _ => palette::BG_ELEVATED,
            })),
            border: Border { color: palette::BORDER_CARD, width: 1.0, radius: 8.0.into() },
            text_color: palette::TEXT_MAIN,
            shadow: Shadow::default(),
            snap: false,
        });

        let url_btn = button(
            row![
                text("🌐").size(12),
                text("Stream URL").size(12).color(palette::TEXT_MAIN),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press(Message::OpenUrlDialog)
        .padding([8, 14])
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => palette::BG_HOVER,
                _ => palette::BG_ELEVATED,
            })),
            border: Border { color: palette::BORDER_CARD, width: 1.0, radius: 8.0.into() },
            text_color: palette::TEXT_MAIN,
            shadow: Shadow::default(),
            snap: false,
        });

        let folder_btn = button(
            row![
                text("📁").size(12),
                text("Select Folder").size(12).color(Color::WHITE),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press(Message::SelectFolder)
        .padding([8, 16])
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => palette::ACCENT_HOVER,
                button::Status::Pressed => palette::ACCENT_PRESSED,
                _ => palette::ACCENT,
            })),
            border: Border { radius: 8.0.into(), ..Default::default() },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        });

        let top_bar = row![
            back_btn,
            Space::new().width(Length::Fixed(8.0)),
            container(text(folder_label).size(11).color(palette::TEXT_DIM))
                .padding([6, 12])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::BG_ELEVATED)),
                    border: Border { radius: 8.0.into(), color: palette::BORDER_CARD, width: 1.0 },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                }),
            Space::new().width(Length::Fill),
            search_input,
            Space::new().width(Length::Fixed(6.0)),
            open_file_btn,
            url_btn,
            folder_btn,
        ]
        .align_y(Alignment::Center)
        .spacing(6)
        .width(Length::Fill);

        // URL Dialog
        let url_panel: Element<'_, Message> = if self.url_dialog_open {
            container(
                row![
                    text("🌐").size(16),
                    text_input("https://example.com/stream.m3u8", &self.url_input)
                        .on_input(Message::UrlInputChanged)
                        .on_submit(Message::PlayUrl)
                        .padding(10)
                        .width(Length::Fill),
                    button(text("Open Stream").size(12).color(Color::WHITE))
                        .on_press(Message::PlayUrl)
                        .padding([8, 18])
                        .style(|_: &Theme, status| button::Style {
                            background: Some(Background::Color(match status {
                                button::Status::Hovered => palette::ACCENT_HOVER,
                                _ => palette::ACCENT,
                            })),
                            border: Border { radius: 8.0.into(), ..Default::default() },
                            text_color: Color::WHITE,
                            shadow: Shadow::default(),
                            snap: false,
                        }),
                    button(text("✕").size(12).color(palette::TEXT_DIM))
                        .on_press(Message::CloseUrlDialog)
                        .padding([8, 12])
                        .style(|_: &Theme, status| button::Style {
                            background: Some(Background::Color(match status {
                                button::Status::Hovered => palette::BG_HOVER,
                                _ => Color::TRANSPARENT,
                            })),
                            border: Border { radius: 8.0.into(), ..Default::default() },
                            text_color: palette::TEXT_DIM,
                            shadow: Shadow::default(),
                            snap: false,
                        }),
                ]
                .align_y(Alignment::Center)
                .spacing(10),
            )
            .width(Length::Fill)
            .padding([12, 16])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::SURFACE)),
                border: Border {
                    color: palette::BORDER_CARD_HOVER,
                    width: 1.0,
                    radius: 12.0.into(),
                },
                shadow: Shadow::default(),
                text_color: Some(palette::TEXT_MAIN),
                snap: false,
            })
            .into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        // Filter videos by search query
        let query = self.search_query.trim().to_lowercase();
        let filtered_videos: Vec<&PathBuf> = self
            .library_videos
            .iter()
            .filter(|p| {
                if query.is_empty() {
                    return true;
                }
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
                name.contains(&query)
            })
            .collect();

        let grid: Element<'_, Message> = if filtered_videos.is_empty() {
            container(
                column![
                    text("🎬").size(40),
                    Space::new().height(Length::Fixed(10.0)),
                    text(if self.library_videos.is_empty() {
                        "No videos in library"
                    } else {
                        "No videos match your search"
                    })
                    .size(16)
                    .color(palette::TEXT_MAIN),
                    Space::new().height(Length::Fixed(4.0)),
                    text("Click 'Select Folder' to load a folder or 'Open File' to play any video directly.")
                        .size(12)
                        .color(palette::TEXT_DIM),
                ]
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
        } else {
            let video_list: Vec<Element<'_, Message>> = filtered_videos
                .into_iter()
                .map(|path| {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("video")
                        .to_string();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("MP4")
                        .to_uppercase();
                    let parent_name = path
                        .parent()
                        .and_then(|d| d.file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    let p = path.clone();

                    // Thumbnail preview container
                    let thumb: Element<'_, Message> = if let Some(handle) = self.thumbnails.get(path) {
                        container(
                            stack![
                                image(handle.clone())
                                    .width(Length::Fixed(190.0))
                                    .height(Length::Fixed(106.0)),
                                container(text(ext.clone()).size(9).color(Color::WHITE))
                                    .padding([2, 6])
                                    .align_x(Alignment::End)
                                    .align_y(Alignment::Start)
                                    .style(|_: &Theme| container::Style {
                                        background: Some(Background::Color(Color { r: 0.0, g: 0.0, b: 0.0, a: 0.75 })),
                                        border: Border { radius: 4.0.into(), ..Default::default() },
                                        shadow: Shadow::default(),
                                        text_color: None,
                                        snap: false,
                                    }),
                            ],
                        )
                        .width(Length::Fixed(190.0))
                        .height(Length::Fixed(106.0))
                        .style(|_: &Theme| container::Style {
                            background: Some(Background::Color(palette::BG_MAIN)),
                            border: Border {
                                color: palette::BORDER_CARD,
                                width: 1.0,
                                radius: 10.0.into(),
                            },
                            shadow: Shadow::default(),
                            text_color: None,
                            snap: false,
                        })
                        .into()
                    } else {
                        container(
                            stack![
                                container(
                                    column![
                                        text("🎬").size(26).color(palette::ACCENT),
                                        text(ext.clone()).size(10).color(palette::TEXT_DIM),
                                    ]
                                    .spacing(4)
                                    .align_x(Alignment::Center),
                                )
                                .width(Length::Fill)
                                .height(Length::Fill)
                                .center_x(Length::Fill)
                                .center_y(Length::Fill),
                            ],
                        )
                        .width(Length::Fixed(190.0))
                        .height(Length::Fixed(106.0))
                        .style(|_: &Theme| container::Style {
                            background: Some(Background::Color(palette::BG_ELEVATED)),
                            border: Border {
                                color: palette::BORDER_CARD,
                                width: 1.0,
                                radius: 10.0.into(),
                            },
                            shadow: Shadow::default(),
                            text_color: None,
                            snap: false,
                        })
                        .into()
                    };

                    // Video Info section
                    let info = column![
                        text(name).size(14).color(palette::TEXT_MAIN),
                        row![
                            text("📁").size(11),
                            text(parent_name).size(11).color(palette::TEXT_DIM),
                            text("•").size(9).color(palette::TEXT_DIM),
                            container(
                                row![
                                    text("🛡").size(9),
                                    text("AI Lookahead Ready").size(9).color(palette::SAFE_GREEN),
                                ]
                                .spacing(3)
                                .align_y(Alignment::Center),
                            )
                            .padding([2, 6])
                            .style(|_: &Theme| container::Style {
                                background: Some(Background::Color(palette::SAFE_GREEN_DIM)),
                                border: Border { radius: 8.0.into(), color: palette::SAFE_GREEN, width: 1.0 },
                                shadow: Shadow::default(),
                                text_color: None,
                                snap: false,
                            }),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    ]
                    .spacing(6)
                    .width(Length::Fill);

                    // Dual Play Action Buttons
                    let p_safe = p.clone();
                    let safe_btn = button(
                        row![
                            text("🛡").size(11),
                            text("Safe Play").size(11).color(Color::WHITE),
                        ]
                        .spacing(5)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::SelectVideoWithMode(p_safe, PlaybackMode::SafeMode))
                    .padding([7, 14])
                    .style(|_: &Theme, status| button::Style {
                        background: Some(Background::Color(match status {
                            button::Status::Hovered => Color::from_rgb(0.08, 0.85, 0.58),
                            _ => palette::SAFE_GREEN,
                        })),
                        border: Border { radius: 16.0.into(), ..Default::default() },
                        text_color: Color::WHITE,
                        shadow: Shadow::default(),
                        snap: false,
                    });

                    let p_instant = p.clone();
                    let instant_btn = button(
                        row![
                            text("⚡").size(11),
                            text("Instant").size(11).color(palette::TEXT_MAIN),
                        ]
                        .spacing(5)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::SelectVideoWithMode(p_instant, PlaybackMode::InstantPlay))
                    .padding([7, 12])
                    .style(|_: &Theme, status| button::Style {
                        background: Some(Background::Color(match status {
                            button::Status::Hovered => palette::BG_HOVER,
                            _ => palette::BG_ELEVATED,
                        })),
                        border: Border { color: palette::BORDER_CARD, width: 1.0, radius: 16.0.into() },
                        text_color: palette::TEXT_MAIN,
                        shadow: Shadow::default(),
                        snap: false,
                    });

                    let prompt_btn = button(
                        text("⋯").size(14).color(palette::TEXT_DIM),
                    )
                    .on_press(Message::OpenPreplayDialog(p))
                    .padding([7, 10])
                    .style(|_: &Theme, status| button::Style {
                        background: Some(Background::Color(match status {
                            button::Status::Hovered => palette::BG_HOVER,
                            _ => Color::TRANSPARENT,
                        })),
                        border: Border { radius: 8.0.into(), ..Default::default() },
                        text_color: palette::TEXT_DIM,
                        shadow: Shadow::default(),
                        snap: false,
                    });

                    let actions = row![safe_btn, instant_btn, prompt_btn]
                        .spacing(6)
                        .align_y(Alignment::Center);

                    // Card container
                    container(
                        row![thumb, info, actions]
                            .align_y(Alignment::Center)
                            .spacing(16)
                            .width(Length::Fill)
                            .padding(12),
                    )
                    .width(Length::Fill)
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(palette::SURFACE_CARD)),
                        border: Border {
                            color: palette::BORDER_CARD,
                            width: 1.0,
                            radius: 12.0.into(),
                        },
                        shadow: Shadow::default(),
                        text_color: None,
                        snap: false,
                    })
                    .into()
                })
                .collect();

            scrollable(column(video_list).spacing(10).padding(4))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        let status_color = if self.status_is_error { palette::ALERT } else { palette::TEXT_DIM };
        let status_bar = row![
            text("●").size(9).color(if self.status_is_error { palette::ALERT } else { palette::ACCENT }),
            text(&self.status).size(11).color(status_color),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        container(
            column![
                top_bar,
                url_panel,
                Space::new().height(Length::Fixed(6.0)),
                status_bar,
                Space::new().height(Length::Fixed(6.0)),
                grid,
            ]
            .spacing(4),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(16)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::BG_MAIN)),
            border: Border::default(),
            shadow: Shadow::default(),
            text_color: Some(palette::TEXT_MAIN),
            snap: false,
        })
        .into()
    }

    fn view_ai_panel(&self) -> Element<'_, Message> {
        if !self.ai_panel_open {
            return Space::new().height(Length::Fixed(0.0)).into();
        }

        // Section header with purple AI accent bar
        let header = row![
            container(
                Space::new().width(Length::Fixed(4.0)).height(Length::Fixed(20.0)),
            )
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::AI_PURPLE)),
                border: Border { radius: 2.0.into(), ..Default::default() },
                shadow: Shadow::default(),
                text_color: None,
                snap: false,
            }),
            text("✨ Gemini Vision Lookahead Moderator").size(15).color(palette::TEXT_MAIN),
            Space::new().width(Length::Fill),
            container(
                row![
                    text("●").size(8).color(if self.gemini_api_key.trim().is_empty() { palette::WARN } else { palette::SAFE_GREEN }),
                    text(if self.gemini_api_key.trim().is_empty() { "API Key Required" } else { "Gemini Ready" })
                        .size(10)
                        .color(palette::TEXT_DIM),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .padding([2, 8])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::BG_ELEVATED)),
                border: Border { radius: 10.0.into(), color: palette::BORDER_CARD, width: 1.0 },
                shadow: Shadow::default(),
                text_color: None,
                snap: false,
            }),
            button(text("✕").size(11).color(palette::TEXT_DIM))
                .on_press(Message::ToggleAiPanel)
                .padding([4, 8])
                .style(|_: &Theme, s| button::Style {
                    background: Some(Background::Color(match s {
                        button::Status::Hovered => palette::BG_HOVER,
                        _ => Color::TRANSPARENT,
                    })),
                    border: Border { radius: 4.0.into(), ..Default::default() },
                    text_color: palette::TEXT_DIM,
                    shadow: Shadow::default(),
                    snap: false,
                }),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        // API Key row
        let api_key_row = row![
            text("Gemini API Key").size(12).color(palette::TEXT_DIM),
            text_input("AIzaSy... (get free key at aistudio.google.com)", &self.gemini_api_key)
                .on_input(Message::GeminiApiKeyChanged)
                .padding(9)
                .width(Length::FillPortion(2))
                .style(|_: &Theme, _| iced::widget::text_input::Style {
                    background: Background::Color(palette::BG_ELEVATED),
                    border: Border {
                        color: palette::BORDER_CARD,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    placeholder: palette::TEXT_DIM,
                    value: palette::TEXT_MAIN,
                    selection: palette::ACCENT_SOFT,
                    icon: palette::TEXT_DIM,
                }),
            container(text("aistudio.google.com").size(10).color(palette::ACCENT))
                .padding([4, 8])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::ACCENT_DIM)),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                }),
        ]
        .align_y(Alignment::Center)
        .spacing(10);

        // Model selector row
        let model_row = row![
            text("Vision Model").size(12).color(palette::TEXT_DIM),
            pick_list(
                &GEMINI_MODELS[..],
                Some(self.gemini_model.as_str()),
                |s: &str| Message::GeminiModelSelected(s.to_string()),
            )
            .placeholder("gemini-3.8-flash")
            .width(Length::Fixed(220.0))
            .style(dark_pick_list_style()),
            text("Stitches 4 frames into 2x2 grid per API request").size(11).color(palette::TEXT_DIM),
        ]
        .align_y(Alignment::Center)
        .spacing(10);

        // Prompt input
        let prompt_section = column![
            text("Moderation Criteria / Skip Prompt").size(12).color(palette::TEXT_DIM),
            text_input(
                "Detect explicit adult nudity, extreme violence, gore, or sponsor segments...",
                &self.ai_skip_prompt,
            )
            .on_input(Message::AiPromptChanged)
            .padding(9)
            .style(|_: &Theme, _| iced::widget::text_input::Style {
                background: Background::Color(palette::BG_ELEVATED),
                border: Border {
                    color: palette::BORDER_CARD,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                placeholder: palette::TEXT_DIM,
                value: palette::TEXT_MAIN,
                selection: palette::ACCENT_SOFT,
                icon: palette::TEXT_DIM,
            }),
        ]
        .spacing(6);

        // Scan button + status
        let scan_btn = button(
            row![
                text(if self.scan_progress.is_some() { "⏳" } else { "✨" }).size(13),
                text(if self.scan_progress.is_some() {
                    "Scanning in Progress..."
                } else {
                    "Run Gemini Lookahead Scan"
                })
                .size(12)
                .color(Color::WHITE),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .on_press_maybe(if self.scan_progress.is_some() {
            None
        } else {
            Some(Message::StartAiScan)
        })
        .padding([8, 20])
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => Color::from_rgb(0.72, 0.40, 1.0),
                _ => palette::AI_PURPLE,
            })),
            border: Border { radius: 10.0.into(), ..Default::default() },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        });

        // Summary chips of cuts found
        let cuts_summary: Element<'_, Message> = if self.unsafe_segments.is_empty() {
            container(text("No explicit segments detected yet").size(11).color(palette::TEXT_DIM))
                .padding([4, 10])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::BG_ELEVATED)),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                })
                .into()
        } else {
            let count = self.unsafe_segments.len();
            let mut cuts_row = vec![
                container(
                    row![
                        text("🔴").size(9),
                        text(format!("{} cuts ready", count)).size(11).color(palette::SKIP_RED),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
                .padding([4, 10])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::SKIP_RED_DIM)),
                    border: Border { radius: 6.0.into(), color: palette::SKIP_RED, width: 1.0 },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                })
                .into(),
            ];
            for (start, end) in self.unsafe_segments.iter().take(4) {
                cuts_row.push(
                    container(
                        text(format!("{}-{}", format_duration_short(*start), format_duration_short(*end)))
                            .size(10)
                            .color(palette::SKIP_RED),
                    )
                    .padding([3, 8])
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(palette::BG_ELEVATED)),
                        border: Border { radius: 6.0.into(), ..Default::default() },
                        shadow: Shadow::default(),
                        text_color: None,
                        snap: false,
                    })
                    .into(),
                );
            }
            row(cuts_row).spacing(6).align_y(Alignment::Center).into()
        };

        let scan_row = row![
            scan_btn,
            Space::new().width(Length::Fixed(8.0)),
            text(self.status.clone())
                .size(11)
                .color(if self.status_is_error { palette::ALERT } else { palette::TEXT_DIM }),
            Space::new().width(Length::Fill),
            cuts_summary,
        ]
        .align_y(Alignment::Center)
        .spacing(8);

        // Progress bar
        let progress_section = if let Some(p) = self.scan_progress {
            column![
                row![
                    text(self.scan_status.clone())
                        .size(11)
                        .color(palette::TEXT_DIM),
                    Space::new().width(Length::Fill),
                    text(format!("{}%", (p * 100.0).round() as u32))
                        .size(11)
                        .color(palette::AI_PURPLE),
                ]
                .align_y(Alignment::Center),
                progress_bar(0.0..=1.0, p)
                    .length(Length::Fill)
                    .girth(Length::Fixed(6.0)),
            ]
            .spacing(4)
        } else {
            column![]
        };

        container(
            column![header, api_key_row, model_row, prompt_section, scan_row, progress_section]
                .spacing(10),
        )
        .width(Length::Fill)
        .padding([14, 20])
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::SURFACE_DOCK)),
            border: Border {
                color: palette::AI_PURPLE_DIM,
                width: 1.0,
                radius: 14.0.into(),
            },
            shadow: Shadow::default(),
            text_color: Some(palette::TEXT_MAIN),
            snap: false,
        })
        .into()
    }

    /// Smart Timeline (The Crown Jewel of Otip Lookahead):
    /// Multi-layer visual timeline:
    /// - Gray: Unscanned/Unknown ahead
    /// - Green: Scanned and safe buffer zone
    /// - Red: Explicit content detected segments (auto-skipped)
    /// - Overlay slider & chapter ticks
    fn render_smart_timeline(&self) -> Element<'_, Message> {
        let total_secs = self.duration.as_secs_f64().max(0.001);
        let pos_secs = self.position.as_secs_f64().clamp(0.0, total_secs);

        // 1. Safe Scanned Buffer calculation:
        // In Safe Mode or when scan completed: full 100% is verified.
        // In Instant Play: from 0 up to current position + lookahead buffer seconds.
        let lookahead_secs = if self.playback_mode == PlaybackMode::SafeMode || (!self.unsafe_segments.is_empty() && self.scan_progress.is_none()) {
            total_secs
        } else {
            (pos_secs + self.buffered_ahead_secs.max(30.0)).min(total_secs)
        };
        let safe_portion = ((lookahead_secs / total_secs) * 1000.0).round().clamp(0.0, 1000.0) as u16;
        let unscanned_portion = 1000 - safe_portion;

        // Visual multi-color buffer track:
        // Emerald Green (Scanned Safe Buffer) + Neutral Zinc (Unscanned Ahead)
        let base_colored_track: Element<'_, Message> = row![
            container(Space::new().width(Length::Fill).height(Length::Fixed(6.0)))
                .width(Length::FillPortion(safe_portion.max(1)))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::SAFE_GREEN)),
                    border: Border { radius: 3.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                }),
            container(Space::new().width(Length::Fill).height(Length::Fixed(6.0)))
                .width(Length::FillPortion(unscanned_portion.max(1)))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::UNSCANNED_GRAY)),
                    border: Border { radius: 3.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                }),
        ]
        .spacing(0)
        .width(Length::Fill)
        .into();

        // 2. Flagged Unsafe Segments (Auto-Skip red markers)
        let skip_markers: Element<'_, Message> = if self.unsafe_segments.is_empty() || self.duration.is_zero() {
            Space::new().height(Length::Fixed(0.0)).into()
        } else {
            let mut markers = Vec::new();
            let mut cur = 0.0;
            for (start, end) in &self.unsafe_segments {
                let s = start.as_secs_f64().clamp(0.0, total_secs);
                let e = end.as_secs_f64().clamp(s, total_secs);
                if s > cur {
                    let gap = s - cur;
                    let portion = ((gap / total_secs) * 1000.0).round().max(1.0) as u16;
                    markers.push(
                        container(Space::new().width(Length::Fill).height(Length::Fixed(6.0)))
                            .width(Length::FillPortion(portion))
                            .style(|_: &Theme| container::Style {
                                background: Some(Background::Color(Color::TRANSPARENT)),
                                ..Default::default()
                            })
                            .into(),
                    );
                }
                let seg_len = (e - s).max(0.1);
                let portion = ((seg_len / total_secs) * 1000.0).round().max(4.0) as u16;
                markers.push(
                    mouse_area(
                        container(Space::new().width(Length::Fill).height(Length::Fixed(6.0)))
                            .width(Length::FillPortion(portion))
                            .style(|_: &Theme| container::Style {
                                background: Some(Background::Color(palette::SKIP_RED)),
                                border: Border {
                                    radius: 3.0.into(),
                                    color: Color::WHITE,
                                    width: 1.0,
                                },
                                shadow: Shadow::default(),
                                text_color: None,
                                snap: false,
                            }),
                    )
                    .on_press(Message::SeekTo(*start))
                    .into(),
                );
                cur = e;
            }
            if cur < total_secs {
                let gap = total_secs - cur;
                let portion = ((gap / total_secs) * 1000.0).round().max(1.0) as u16;
                markers.push(
                    container(Space::new().width(Length::Fill).height(Length::Fixed(6.0)))
                        .width(Length::FillPortion(portion))
                        .style(|_: &Theme| container::Style {
                            background: Some(Background::Color(Color::TRANSPARENT)),
                            ..Default::default()
                        })
                        .into(),
                );
            }
            row(markers).spacing(0).width(Length::Fill).into()
        };

        // 3. Chapters
        let chapter_segments = chapter_segments(&self.chapters, self.duration);
        let chapter_strip: Element<'_, Message> = if chapter_segments.is_empty() {
            Space::new().height(Length::Fixed(0.0)).into()
        } else {
            let position = self.position;
            let segments: Vec<Element<'_, Message>> = chapter_segments
                .into_iter()
                .map(|(start, end)| {
                    let gap = (end - start).as_secs_f64().max(0.0);
                    let portion = ((gap / total_secs) * 1000.0).round().clamp(1.0, 1000.0) as u16;
                    let active = position >= start && position < end;
                    mouse_area(
                        container(Space::new().width(Length::Fill).height(Length::Fixed(4.0)))
                            .width(Length::FillPortion(portion))
                            .style(move |_: &Theme| container::Style {
                                background: Some(Background::Color(if active {
                                    palette::ACCENT
                                } else {
                                    palette::MARKER_IDLE
                                })),
                                border: Border { radius: 2.0.into(), ..Default::default() },
                                shadow: Shadow::default(),
                                text_color: None,
                                snap: false,
                            }),
                    )
                    .on_press(Message::SeekTo(start))
                    .into()
                })
                .collect();
            row(segments).spacing(2).width(Length::Fill).into()
        };

        // 4. Seek Slider
        let seek_bar = slider(0.0..=1.0, self.timeline_pos as f64, Message::Seek)
            .step(0.002)
            .width(Length::Fill)
            .style(dark_slider_style());

        // Stack seek bar directly with the underlying colored smart timeline tracks
        let layered_timeline = stack![
            container(base_colored_track).width(Length::Fill).align_y(Alignment::Center),
            container(skip_markers).width(Length::Fill).align_y(Alignment::Center),
            container(seek_bar).width(Length::Fill).align_y(Alignment::Center),
        ]
        .width(Length::Fill);

        // Smart Timeline Legend & Metrics Bar
        let time_text = format!("{} / {}", format_duration_short(self.position), format_duration_short(self.duration));
        let chapter_info = current_chapter_title(&self.chapters, self.position);

        let chapter_chip: Element<'_, Message> = if let Some(ch) = chapter_info {
            container(text(format!("·  {}", ch)).size(11).color(palette::ACCENT))
                .padding([1, 6])
                .style(|_: &Theme| container::Style::default())
                .into()
        } else {
            Space::new().width(Length::Shrink).into()
        };

        let time_chip = row![
            text(time_text).size(12).color(palette::TEXT_MAIN),
            chapter_chip,
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let skips_count = self.unsafe_segments.len();
        let legend = row![
            row![
                container(Space::new().width(Length::Fixed(8.0)).height(Length::Fixed(8.0)))
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(palette::SAFE_GREEN)),
                        border: Border { radius: 4.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                text("Scanned Safe").size(10).color(palette::TEXT_DIM),
            ].spacing(4).align_y(Alignment::Center),
            row![
                container(Space::new().width(Length::Fixed(8.0)).height(Length::Fixed(8.0)))
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(palette::SKIP_RED)),
                        border: Border { radius: 4.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                text(if skips_count > 0 {
                    format!("Auto-Skip ({} cuts)", skips_count)
                } else {
                    "Explicit Skip".into()
                })
                .size(10)
                .color(palette::SKIP_RED),
            ].spacing(4).align_y(Alignment::Center),
            row![
                container(Space::new().width(Length::Fixed(8.0)).height(Length::Fixed(8.0)))
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(palette::UNSCANNED_GRAY)),
                        border: Border { radius: 4.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                text("Unscanned").size(10).color(palette::TEXT_DIM),
            ].spacing(4).align_y(Alignment::Center),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let buffer_label = match self.playback_mode {
            PlaybackMode::SafeMode => "🛡 Fully Protected",
            PlaybackMode::InstantPlay => "⚡ Lookahead: +30s Buffer",
            PlaybackMode::AutoSkip => "🤖 Real-Time Skip Active",
        };
        let buffer_chip = container(text(buffer_label).size(10).color(palette::SAFE_GREEN))
            .padding([2, 8])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::SAFE_GREEN_DIM)),
                border: Border { radius: 10.0.into(), color: palette::SAFE_GREEN, width: 1.0 },
                shadow: Shadow::default(),
                text_color: None,
                snap: false,
            });

        let timeline_header = row![
            time_chip,
            Space::new().width(Length::Fill),
            legend,
            Space::new().width(Length::Fill),
            buffer_chip,
        ]
        .align_y(Alignment::Center)
        .width(Length::Fill);

        column![
            chapter_strip,
            layered_timeline,
            Space::new().height(Length::Fixed(2.0)),
            timeline_header,
        ]
        .spacing(4)
        .width(Length::Fill)
        .into()
    }

    fn view_player(&self) -> Element<'_, Message> {
        let video_area: Element<'_, Message> = if let Some(handle) = &self.video_handle {
            iced::widget::image(handle.clone()).width(Length::Fill).height(Length::Fill).into()
        } else {
            let display_name = self
                .selected_video_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
                .or_else(|| self.stream_title.clone())
                .unwrap_or("Ready to play".into());

            container(
                column![
                    container(text("◈").size(36).color(palette::ACCENT))
                        .padding(14)
                        .style(|_: &Theme| container::Style {
                            background: Some(Background::Color(palette::ACCENT_DIM)),
                            border: Border { radius: 36.0.into(), color: palette::ACCENT, width: 1.0 },
                            shadow: Shadow::default(),
                            text_color: None,
                            snap: false,
                        }),
                    Space::new().height(Length::Fixed(12.0)),
                    text(display_name).size(16).color(palette::TEXT_MAIN),
                    Space::new().height(Length::Fixed(4.0)),
                    text(if self.video_player.is_some() {
                        "Initializing video engine (SW Fallback)..."
                    } else {
                        "Select a video from the library to begin"
                    })
                    .size(12)
                    .color(palette::TEXT_DIM),
                ]
                .align_x(Alignment::Center)
                .spacing(4),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(Color { r: 0.05, g: 0.05, b: 0.06, a: 1.0 })),
                border: Border::default(),
                shadow: Shadow::default(),
                text_color: None,
                snap: false,
            })
            .into()
        };

        // Top-Right Mode Badge & Menu Overlay
        let (mode_icon, mode_txt, mode_col) = match self.playback_mode {
            PlaybackMode::SafeMode => ("🛡", "Safe Mode", palette::SAFE_GREEN),
            PlaybackMode::InstantPlay => ("⚡", "Instant Play", palette::WARN),
            PlaybackMode::AutoSkip => ("🤖", "Auto-Skip", palette::ACCENT),
        };

        let mode_chip = button(
            row![
                text(mode_icon).size(12),
                text(mode_txt).size(11).color(mode_col),
                text("▾").size(10).color(palette::TEXT_DIM),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .on_press(Message::ToggleModeMenu)
        .padding([6, 12])
        .style(move |_: &Theme, _| button::Style {
            background: Some(Background::Color(if self.mode_menu_open {
                palette::BG_HOVER
            } else {
                palette::SURFACE_DOCK
            })),
            border: Border {
                radius: 12.0.into(),
                color: mode_col,
                width: 1.0,
            },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        });

        let mode_choice = |label: &'static str, hint: &'static str, mode: PlaybackMode, current: PlaybackMode| -> Element<'_, Message> {
            row![
                overlay_btn(label, mode, current == mode),
                text(hint).size(11).color(palette::TEXT_DIM),
            ]
            .align_y(Alignment::Center)
            .spacing(8)
            .into()
        };
        let current_mode = self.playback_mode;
        let mode_panel: Element<'_, Message> = if self.mode_menu_open {
            container(
                column![
                    mode_choice("Safe Mode", "Look ahead & skip sensitive scenes", PlaybackMode::SafeMode, current_mode),
                    mode_choice("Instant Play", "Start right away, live background scan", PlaybackMode::InstantPlay, current_mode),
                    mode_choice("Auto-Skip", "Skip flagged segments on-the-fly", PlaybackMode::AutoSkip, current_mode),
                ]
                .spacing(6),
            )
            .padding(12)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::PANEL_BG)),
                border: Border {
                    color: palette::DIVIDER,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                shadow: Shadow::default(),
                text_color: Some(palette::TEXT_MAIN),
                snap: false,
            })
            .into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        let top_overlay = container(
            column![
                row![
                    Space::new().width(Length::Fill),
                    mode_chip,
                ],
                mode_panel,
            ]
            .spacing(6),
        )
        .padding(12);

        let stacked_video = stack![
            container(video_area).width(Length::Fill).height(Length::Fill),
            container(top_overlay).width(Length::Fill).height(Length::Fill).align_x(Alignment::End).align_y(Alignment::Start),
        ]
        .width(Length::Fill)
        .height(Length::FillPortion(1));

        // Transport & Controls setup
        let play_pause_icon = if self.is_playing { "⏸" } else { "▶" };

        let left_group = row![
            button(
                row![
                    text("←").size(12).color(palette::TEXT_DIM),
                    text("Library").size(11).color(palette::TEXT_DIM),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .on_press(Message::NavigateTo(AppScreen::Library))
            .padding([6, 12])
            .style(|_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => palette::BG_HOVER,
                    _ => Color::TRANSPARENT,
                })),
                border: Border { radius: 8.0.into(), ..Default::default() },
                text_color: palette::TEXT_DIM,
                shadow: Shadow::default(),
                snap: false,
            }),
            Space::new().width(Length::Fixed(6.0)),
            // Volume
            ctrl_btn_icon(
                if self.is_muted || self.volume < 0.01 { "🔇" } else if self.volume < 0.5 { "🔉" } else { "🔊" },
                Message::ToggleMute,
                false,
            ),
            slider(0.0..=1.0, self.volume as f64, Message::SetVolume)
                .step(0.02)
                .width(Length::Fixed(75.0))
                .style(dark_slider_style()),
        ]
        .spacing(4)
        .align_y(Alignment::Center);

        let center_group = row![
            ctrl_btn_icon("⏮", Message::PrevVideo, false),
            button(
                row![
                    text("↺").size(12).color(palette::TEXT_MAIN),
                    text("10s").size(10).color(palette::TEXT_MAIN),
                ]
                .spacing(2)
                .align_y(Alignment::Center),
            )
            .on_press(Message::SkipBackward)
            .padding([6, 8])
            .style(|_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => palette::BG_HOVER,
                    _ => Color::TRANSPARENT,
                })),
                border: Border { radius: 8.0.into(), ..Default::default() },
                text_color: palette::TEXT_MAIN,
                shadow: Shadow::default(),
                snap: false,
            }),
            button(
                container(text(play_pause_icon).size(20).color(Color::WHITE))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .on_press(Message::PlayPause)
            .padding(0)
            .width(Length::Fixed(46.0))
            .height(Length::Fixed(46.0))
            .style(|_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => palette::ACCENT_HOVER,
                    button::Status::Pressed => palette::ACCENT_PRESSED,
                    _ => palette::ACCENT,
                })),
                border: Border { radius: 23.0.into(), ..Default::default() },
                text_color: Color::WHITE,
                shadow: Shadow::default(),
                snap: false,
            }),
            button(
                row![
                    text("10s").size(10).color(palette::TEXT_MAIN),
                    text("↻").size(12).color(palette::TEXT_MAIN),
                ]
                .spacing(2)
                .align_y(Alignment::Center),
            )
            .on_press(Message::SkipForward)
            .padding([6, 8])
            .style(|_: &Theme, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => palette::BG_HOVER,
                    _ => Color::TRANSPARENT,
                })),
                border: Border { radius: 8.0.into(), ..Default::default() },
                text_color: palette::TEXT_MAIN,
                shadow: Shadow::default(),
                snap: false,
            }),
            ctrl_btn_icon("⏭", Message::NextVideo, false),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let speed_menu = pick_list(
            &SPEED_OPTIONS[..],
            Some(speed_label(self.playback_speed)),
            |label: &'static str| Message::SetSpeed(parse_speed_label(label)),
        )
        .placeholder("1.0x")
        .width(Length::Fixed(72.0))
        .style(dark_pick_list_style());

        let cc_btn = ctrl_btn_label("CC", Message::ToggleCaptions, self.show_subs);
        let loop_btn = ctrl_btn_icon("🔁", Message::ToggleLoop, self.is_looping);
        let ai_btn = button(
            row![
                text("✨").size(12),
                text("AI Moderator").size(11).color(palette::AI_PURPLE),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .on_press(Message::ToggleAiPanel)
        .padding([6, 12])
        .style(|_: &Theme, status| button::Style {
            background: Some(Background::Color(if self.ai_panel_open {
                palette::AI_PURPLE_DIM
            } else {
                match status {
                    button::Status::Hovered => palette::BG_HOVER,
                    _ => Color::TRANSPARENT,
                }
            })),
            border: Border {
                radius: 12.0.into(),
                color: if self.ai_panel_open { palette::AI_PURPLE } else { palette::BORDER_CARD },
                width: 1.0,
            },
            text_color: palette::AI_PURPLE,
            shadow: Shadow::default(),
            snap: false,
        });

        let settings_btn = ctrl_btn_icon("⚙", Message::ToggleSettings, self.settings_open);
        let pip_btn = ctrl_btn_icon("❏", Message::ToggleMini, self.is_mini);
        let fs_label = if self.is_fullscreen { "🗗" } else { "⛶" };
        let fs_btn = ctrl_btn_icon(fs_label, Message::ToggleFullscreen, false);

        let right_group = row![
            cc_btn,
            loop_btn,
            speed_menu,
            ai_btn,
            settings_btn,
            pip_btn,
            fs_btn,
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let controls_row = row![
            left_group,
            Space::new().width(Length::Fill),
            center_group,
            Space::new().width(Length::Fill),
            right_group,
        ]
        .align_y(Alignment::Center)
        .width(Length::Fill);

        let bottom_bar: Element<'_, Message> = container(
            column![
                self.render_smart_timeline(),
                Space::new().height(Length::Fixed(6.0)),
                controls_row,
            ]
            .spacing(4),
        )
        .width(Length::Fill)
        .padding([12, 18])
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::SURFACE_DOCK)),
            border: Border {
                color: palette::BORDER_CARD,
                width: 1.0,
                radius: 16.0.into(),
            },
            shadow: Shadow::default(),
            text_color: Some(palette::TEXT_MAIN),
            snap: false,
        })
        .into();

        let overlay_controls: Element<'_, Message> = if self.controls_visible {
            container(bottom_bar).width(Length::Fill).padding(12).into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        // Settings Panel
        let chapter_count = self.chapters.len();
        let sub_hint = if self.sub_tracks.is_empty() {
            "No embedded subtitles in this file"
        } else {
            "Embedded tracks from file"
        };
        let settings_panel: Element<'_, Message> = if self.settings_open {
            container(
                column![
                    row![
                        text("Quality").size(12).color(palette::TEXT_MAIN),
                        pick_list(
                            &RenderQuality::ALL[..],
                            Some(self.render_quality),
                            Message::QualitySelected,
                        )
                        .placeholder("360p")
                        .width(Length::Fixed(96.0))
                        .style(dark_pick_list_style()),
                        text("SW render — higher is sharper, uses more CPU")
                            .size(11)
                            .color(palette::TEXT_DIM),
                        Space::new().width(Length::Fill),
                        text(if chapter_count == 0 {
                            "No embedded chapters".to_string()
                        } else {
                            format!(
                                "{} embedded chapter{} (click markers to jump)",
                                chapter_count,
                                if chapter_count == 1 { "" } else { "s" }
                            )
                        })
                        .size(11)
                        .color(palette::TEXT_DIM),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(10),
                    row![
                        text("Subtitles").size(12).color(palette::TEXT_MAIN),
                        pick_list(
                            &self.subtitle_options[..],
                            Some(self.selected_subtitle.clone()),
                            Message::SubtitleSelected,
                        )
                        .placeholder("Off")
                        .width(Length::Fixed(200.0))
                        .style(dark_pick_list_style()),
                        text(sub_hint)
                            .size(11)
                            .color(palette::TEXT_DIM),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(10),
                ]
                .spacing(8),
            )
            .width(Length::Fill)
            .padding([12, 16])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::PANEL_BG)),
                border: Border {
                    color: palette::DIVIDER,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                shadow: Shadow::default(),
                text_color: Some(palette::TEXT_MAIN),
                snap: false,
            })
            .into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        let ai_panel = self.view_ai_panel();

        let player_stack = column![stacked_video, settings_panel, ai_panel, overlay_controls]
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill);

        mouse_area(player_stack).on_move(|_| Message::MouseMoved).into()
    }
}

/// Playback-speed options shown in the speed `pick_list` dropdown.
const SPEED_OPTIONS: [&str; 4] = ["0.5x", "1.0x", "1.5x", "2.0x"];

/// Gemini models available for selection.
const GEMINI_MODELS: [&str; 4] = [
    "gemini-3.8-flash",
    "gemini-2.0-flash",
    "gemini-1.5-flash-latest",
    "gemini-3.5-flash-lite",
];

/// Current speed → dropdown label (nearest option shown as selected).
fn speed_label(speed: f32) -> &'static str {
    if (speed - 0.5).abs() < 0.01 {
        "0.5x"
    } else if (speed - 1.5).abs() < 0.01 {
        "1.5x"
    } else if (speed - 2.0).abs() < 0.01 {
        "2.0x"
    } else {
        "1.0x"
    }
}

/// Dropdown label → speed value for `Message::SetSpeed(f32)` -> `set_rate`.
fn parse_speed_label(label: &str) -> f32 {
    label
        .trim_end_matches('x')
        .parse::<f32>()
        .unwrap_or(1.0)
        .clamp(0.25, 4.0)
}

/// Shared dark-chrome style for dropdown (`pick_list`) controls so the speed
/// and quality menus match the control-bar buttons.
fn dark_pick_list_style(
) -> impl Fn(&Theme, iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    |_: &Theme, _| iced::widget::pick_list::Style {
        text_color: palette::TEXT_MAIN,
        placeholder_color: palette::TEXT_DIM,
        handle_color: palette::TEXT_DIM,
        background: Background::Color(palette::BTN_BG),
        border: Border {
            radius: 8.0.into(),
            color: palette::BORDER_SUBTLE,
            width: 1.0,
        },
    }
}

/// Shared dark-chrome style for `slider` controls (seek + volume): accent
/// selected rail and handle, dim unselected track.
fn dark_slider_style(
) -> impl Fn(&Theme, iced::widget::slider::Status) -> iced::widget::slider::Style {
    |_: &Theme, _| iced::widget::slider::Style {
        rail: iced::widget::slider::Rail {
            backgrounds: (
                Background::Color(palette::ACCENT),
                Background::Color(palette::TRACK_BG),
            ),
            width: 5.0,
            border: Border::default(),
        },
        handle: iced::widget::slider::Handle {
            shape: iced::widget::slider::HandleShape::Circle { radius: 7.0.into() },
            background: Background::Color(palette::ACCENT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// Compact icon-only control button with accent highlight when active.
fn ctrl_btn_icon<'a>(icon: &'a str, msg: Message, active: bool) -> Element<'a, Message> {
    button(text(icon).size(14).color(if active { palette::ACCENT } else { palette::TEXT_MAIN }))
        .on_press(msg)
        .padding([6, 10])
        .style(move |_: &Theme, status| button::Style {
            background: Some(Background::Color(match (active, status) {
                (true, _) => palette::ACCENT_DIM,
                (_, button::Status::Hovered) => palette::BG_HOVER,
                _ => Color::TRANSPARENT,
            })),
            border: Border { radius: 8.0.into(), ..Default::default() },
            text_color: if active { palette::ACCENT } else { palette::TEXT_MAIN },
            shadow: Shadow::default(),
            snap: false,
        })
        .into()
}

/// Compact label control button with accent highlight when active.
fn ctrl_btn_label<'a>(label: &'a str, msg: Message, active: bool) -> Element<'a, Message> {
    button(text(label).size(11).color(if active { palette::ACCENT } else { palette::TEXT_MAIN }))
        .on_press(msg)
        .padding([6, 10])
        .style(move |_: &Theme, status| button::Style {
            background: Some(Background::Color(match (active, status) {
                (true, _) => palette::ACCENT_DIM,
                (_, button::Status::Hovered) => palette::BG_HOVER,
                _ => Color::TRANSPARENT,
            })),
            border: Border { radius: 8.0.into(), ..Default::default() },
            text_color: if active { palette::ACCENT } else { palette::TEXT_MAIN },
            shadow: Shadow::default(),
            snap: false,
        })
        .into()
}

/// Title of the chapter containing `pos` (chapters sorted by start time).
fn current_chapter_title(chapters: &[ChapterInfo], pos: Duration) -> Option<&str> {
    chapters
        .iter()
        .filter(|c| c.time <= pos)
        .next_back()
        .map(|c| c.title.as_str())
}

/// Clickable chapter segments `(start, end)` clipped to `duration`.
/// Empty when there are no chapters (or no duration) — the marker strip
/// hides itself. Zero-length segments are dropped.
fn chapter_segments(chapters: &[ChapterInfo], duration: Duration) -> Vec<(Duration, Duration)> {
    if chapters.is_empty() || duration.is_zero() {
        return Vec::new();
    }
    let mut bounds: Vec<Duration> = chapters.iter().map(|c| c.time.min(duration)).collect();
    bounds.sort();
    bounds.dedup();
    let mut segments = Vec::new();
    let mut prev = Duration::ZERO;
    for bound in bounds {
        if bound > prev {
            segments.push((prev, bound));
        }
        prev = prev.max(bound);
    }
    if prev < duration {
        segments.push((prev, duration));
    }
    segments
}

/// Subtitle picker choice: captions off, or one embedded track by mpv sid.
#[derive(Debug, Clone, PartialEq)]
pub enum SubtitleOption {
    Off,
    Track { id: i64, label: String },
}

impl std::fmt::Display for SubtitleOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubtitleOption::Off => f.write_str("Off"),
            SubtitleOption::Track { label, .. } => f.write_str(label),
        }
    }
}

/// Build picker options from freshly loaded tracks: always `Off` first,
/// then one labeled entry per track.
fn build_subtitle_options(tracks: &[SubTrackInfo]) -> Vec<SubtitleOption> {
    let mut options = vec![SubtitleOption::Off];
    for track in tracks {
        let label = if track.lang.is_empty() || track.lang == "und" {
            track.title.clone()
        } else {
            format!("{} [{}]", track.title, track.lang)
        };
        options.push(SubtitleOption::Track { id: track.id, label });
    }
    options
}

/// Accepts direct http(s) media URLs and HLS/DASH playlists. Watch-page URLs
/// (e.g. youtube.com/watch) need an external extractor and are rejected with
/// a hint instead of failing silently inside mpv.
fn is_playable_url(text: &str) -> bool {
    let url = text.trim();
    (url.starts_with("http://") || url.starts_with("https://")) && url.len() > 12
}

/// Short display name for a stream: host + truncated path, for the window
/// title and the player placeholder.
fn stream_display_name(url: &str) -> String {
    let url = url.trim();
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let host = without_scheme.split('/').next().unwrap_or(without_scheme);
    if host.is_empty() {
        return "Network stream".to_string();
    }
    const MAX: usize = 48;
    if without_scheme.chars().count() > MAX {
        let truncated: String = without_scheme.chars().take(MAX).collect();
        format!("{}…", truncated)
    } else {
        without_scheme.to_string()
    }
}

fn overlay_btn<'a>(label: &'a str, mode: PlaybackMode, active: bool) -> Element<'a, Message> {
    button(text(label).size(11).align_x(Alignment::Center))
        .on_press(Message::SetPlaybackMode(mode))
        .padding([6, 14])
        .style(move |_: &Theme, status| button::Style {
            background: Some(Background::Color(match (active, status) {
                (true, _) => palette::ACCENT,
                (_, button::Status::Hovered) => palette::BG_HOVER,
                _ => palette::BTN_BG,
            })),
            border: Border { radius: 20.0.into(), ..Default::default() },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        })
        .into()
}

async fn scan_default_media_dirs() -> Vec<PathBuf> {
    tokio::task::spawn_blocking(|| {
        let mut roots = Vec::new();
        if let Some(p) = dirs::video_dir() { roots.push(p); }
        if let Some(p) = dirs::download_dir() { roots.push(p); }
        if roots.is_empty() { if let Some(h) = dirs::home_dir() { roots.push(h); } }
        let mut videos = Vec::new();
        const EXTS: &[&str] = &["mp4","mkv","avi","mov","webm","flv","wmv","m4v","mpg","mpeg"];
        for root in &roots {
            if !root.exists() { continue; }
            for entry in walkdir::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
                let p = entry.path();
                if !p.is_file() { continue; }
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    if EXTS.contains(&ext.to_ascii_lowercase().as_str()) { videos.push(p.to_path_buf()); }
                }
            }
        }
        videos.sort(); if videos.len()>500 { videos.truncate(500); }
        tracing::info!("Auto-scanned {:?} -> {} videos", roots, videos.len());
        videos
    }).await.unwrap_or_default()
}

// ── Boilerplate ─────────────────────────────────────────────────────
fn boot() -> (OtipApp, Task<Message>) { OtipApp::new() }
fn update(app: &mut OtipApp, msg: Message) -> Task<Message> { app.update(msg) }
fn view(app: &OtipApp) -> Element<'_, Message> { app.view() }
fn theme(_: &OtipApp) -> Theme { Theme::Dark }
fn title(app: &OtipApp) -> String { app.title() }
fn subscription(app: &OtipApp) -> iced::Subscription<Message> {
    let window_opened = iced::window::open_events().map(Message::WindowOpened);
    // 4. Backend wiring helpers + 3. Keyboard shortcuts via events_with + 2. Auto-hide tick
    let frames = iced::Subscription::run(|| {
        iced::stream::channel::<Message>(32, move |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            loop {
                // ── Non-blocking drain (deadlock fix) ────────────────────
                // 1. Snapshot the receiver Arc with try_lock and drop the
                //    global guard IMMEDIATELY — never hold it across an await
                //    or while taking the inner receiver lock (nested blocking
                //    locks here stalled both the executor and the UI thread,
                //    freezing buttons and newly added elements after a moment).
                // 2. try_lock (not lock) the inner receiver so a contended
                //    tick is simply skipped instead of blocking the executor.
                // 3. Drain with a cap, keep only the latest frame/position
                //    (channel-flooding fix: 30fps producer vs 60fps poller
                //    must not queue unbounded work), drop the guard, THEN
                //    await on sends.
                let rx_arc_opt = video_player::snapshot_frame_receiver();
                // Latest-only coalescing for every event kind (channel-flood
                // safe: the producer only ever gets fresher).
                let mut latest_chapters: Option<Vec<ChapterInfo>> = None;
                let mut latest_subs: Option<Vec<SubTrackInfo>> = None;
                let mut latest_cache: Option<f64> = None;
                let mut latest_buffering: Option<bool> = None;
                let (frame, pos_update) = if let Some(rx_arc) = rx_arc_opt {
                    match rx_arc.try_lock() {
                        Ok(mut guard) => {
                            let mut latest_frame: Option<Handle> = None;
                            let mut latest_pos: Option<(Duration, Duration)> = None;
                            // Cap drained events per tick: the bounded channel
                            // holds at most MAX_QUEUED_EVENTS, so this always
                            // fully drains while staying bounded if the
                            // producer ever outruns us.
                            for _ in 0..16 {
                                match guard.try_recv() {
                                    Ok(ev) => match ev {
                                        PlayerEvent::Frame(h) => latest_frame = Some(h),
                                        PlayerEvent::PositionUpdate { position, duration } => latest_pos = Some((position, duration)),
                                        PlayerEvent::Chapters(ch) => latest_chapters = Some(ch),
                                        PlayerEvent::SubTracks(subs) => latest_subs = Some(subs),
                                        PlayerEvent::CacheUpdate { readahead_secs } => latest_cache = Some(readahead_secs),
                                        PlayerEvent::Buffering(buffering) => latest_buffering = Some(buffering),
                                        PlayerEvent::StateChanged(_playing) => {},
                                        PlayerEvent::VolumeChanged(_) => {},
                                        PlayerEvent::Ready { .. } => {},
                                        PlayerEvent::Error(e) => tracing::warn!("PlayerEvent::Error in subscription: {}", e),
                                    },
                                    Err(_) => break, // Empty or Disconnected: nothing more to drain
                                }
                            }
                            // Guard drops here with latest-only values; anything
                            // still queued waits for the next 16ms tick.
                            (latest_frame, latest_pos)
                        }
                        Err(_) => (None, None), // contended: skip tick, retry in 16ms
                    }
                } else {
                    (None, None)
                };
                // Guards dropped above — the awaits below hold NO locks.
                if let Some(h) = frame {
                    if out.send(Message::FrameReady(h)).await.is_err() { break; }
                }
                if let Some(chapters) = latest_chapters.take() {
                    if out.send(Message::ChaptersLoaded(chapters)).await.is_err() { break; }
                }
                if let Some(subs) = latest_subs.take() {
                    if out.send(Message::SubTracksLoaded(subs)).await.is_err() { break; }
                }
                if let Some(readahead) = latest_cache.take() {
                    if out.send(Message::CacheUpdate(readahead)).await.is_err() { break; }
                }
                if let Some(buffering) = latest_buffering.take() {
                    if out.send(Message::Buffering(buffering)).await.is_err() { break; }
                }
                if let Some((pos, dur)) = pos_update {
                    if out.send(Message::PositionUpdate(pos, dur)).await.is_err() { break; }
                }
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            }
        })
    });
    let close_requests = iced::window::close_requests().map(|_id| Message::CloseRequested);
    // 2. Auto-hide tick: check every 200ms if 3s elapsed since last mouse move
    let tick = iced::time::every(Duration::from_millis(200)).map(Message::Tick);
    // 3. Mouse movement (always on): drives control auto-hide reveal.
    let mouse_events = iced::event::listen_with(|event, _status, _window| match event {
        Event::Mouse(mouse::Event::CursorMoved { .. }) => Some(Message::MouseMoved),
        _ => None,
    });
    // 4. Keyboard shortcuts (Space/arrows/F/M/C/L). Suspended while the URL
    // dialog is open so typing a stream address never toggles playback.
    // Spec: Space Play/Pause, Left/Right 5s seek, Up/Down volume 10%, F fullscreen
    let key_events = if app.url_dialog_open {
        iced::Subscription::none()
    } else {
        iced::event::listen_with(|event, _status, _window| {
            match event {
                Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                    if modifiers.command() || modifiers.control() {
                        return None;
                    }
                    match key.as_ref() {
                        keyboard::Key::Named(Named::Space) => Some(Message::PlayPause),
                        keyboard::Key::Named(Named::ArrowLeft) => Some(Message::SeekRelative(-5.0)),
                        keyboard::Key::Named(Named::ArrowRight) => Some(Message::SeekRelative(5.0)),
                        keyboard::Key::Named(Named::ArrowUp) => Some(Message::VolumeUp),
                        keyboard::Key::Named(Named::ArrowDown) => Some(Message::VolumeDown),
                        keyboard::Key::Character("f") | keyboard::Key::Character("F") => Some(Message::ToggleFullscreen),
                        keyboard::Key::Character("m") | keyboard::Key::Character("M") => Some(Message::ToggleMute),
                        keyboard::Key::Character("c") | keyboard::Key::Character("C") => Some(Message::ToggleCaptions),
                        keyboard::Key::Character("l") | keyboard::Key::Character("L") => Some(Message::ToggleLoop),
                        _ => None,
                    }
                }
                _ => None,
            }
        })
    };
    // NOTE: no 60fps Noop redraw pump. FrameReady/PositionUpdate messages
    // already drive re-renders when a new frame actually arrives; a blind
    // 60Hz Noop forced a full re-render (and 1MB texture upload) every 16ms
    // even with no new content, saturating the UI thread so buttons and newly
    // added elements stopped responding — the reported freeze.
    iced::Subscription::batch(vec![frames, close_requests, window_opened, tick, mouse_events, key_events])
}

fn main() -> iced::Result {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env().add_directive("otip=info".parse().unwrap())).with_target(false).init();
    tracing::info!("Starting Otip — Splash → Library (thumbnails at 5s) → Player (playbin + controls)");
    iced::application(boot, update, view).theme(theme).title(title).subscription(subscription)
        .window(iced::window::Settings{ size: iced::Size::new(1280.0, 720.0), min_size: Some(iced::Size::new(900.0,600.0)), decorations: false, ..Default::default() }).run()
}

#[cfg(test)]
mod player_controls_tests {
    use super::*;

    #[test]
    fn speed_labels_roundtrip() {
        for label in SPEED_OPTIONS {
            let value = parse_speed_label(label);
            assert_eq!(speed_label(value), label, "label {} must roundtrip", label);
        }
        assert!((parse_speed_label("1.5x") - 1.5).abs() < f32::EPSILON);
        assert!((parse_speed_label("bogus") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn neighbor_video_navigation() {
        let (mut app, _) = OtipApp::new();
        let vids = vec![
            PathBuf::from("a.mp4"),
            PathBuf::from("b.mp4"),
            PathBuf::from("c.mp4"),
        ];
        app.library_videos = vids;
        app.selected_video_path = Some(PathBuf::from("b.mp4"));
        assert_eq!(app.neighbor_video(1), Some(PathBuf::from("c.mp4")));
        assert_eq!(app.neighbor_video(-1), Some(PathBuf::from("a.mp4")));
        // Boundaries no-op instead of panicking.
        app.selected_video_path = Some(PathBuf::from("c.mp4"));
        assert_eq!(app.neighbor_video(1), None);
        app.selected_video_path = Some(PathBuf::from("a.mp4"));
        assert_eq!(app.neighbor_video(-1), None);
        // Unknown selection and empty selection no-op.
        app.selected_video_path = Some(PathBuf::from("z.mp4"));
        assert_eq!(app.neighbor_video(1), None);
        app.selected_video_path = None;
        assert_eq!(app.neighbor_video(1), None);
        app.library_videos.clear();
        assert_eq!(app.neighbor_video(1), None);
    }

    #[test]
    fn toggle_loop_flips_state() {
        let (mut app, _) = OtipApp::new();
        assert!(!app.is_looping);
        let _ = app.update(Message::ToggleLoop);
        assert!(app.is_looping);
        let _ = app.update(Message::ToggleLoop);
        assert!(!app.is_looping);
    }

    fn test_chapters() -> Vec<ChapterInfo> {
        vec![
            ChapterInfo { title: "Intro".into(), time: Duration::from_secs(0) },
            ChapterInfo { title: "Middle".into(), time: Duration::from_secs(60) },
            ChapterInfo { title: "End".into(), time: Duration::from_secs(120) },
        ]
    }

    #[test]
    fn chapter_title_tracks_position() {
        let chapters = test_chapters();
        assert_eq!(current_chapter_title(&chapters, Duration::from_secs(0)), Some("Intro"));
        assert_eq!(current_chapter_title(&chapters, Duration::from_secs(59)), Some("Intro"));
        assert_eq!(current_chapter_title(&chapters, Duration::from_secs(60)), Some("Middle"));
        assert_eq!(current_chapter_title(&chapters, Duration::from_secs(999)), Some("End"));
        assert_eq!(current_chapter_title(&[], Duration::from_secs(10)), None);
    }

    #[test]
    fn chapter_segments_cover_duration_without_gaps() {
        let duration = Duration::from_secs(180);
        let segments = chapter_segments(&test_chapters(), duration);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], (Duration::from_secs(0), Duration::from_secs(60)));
        assert_eq!(segments[2], (Duration::from_secs(120), duration));
        // Contiguous: each segment starts where the previous ended.
        for pair in segments.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
        // Degenerate input hides the strip instead of breaking layout.
        assert!(chapter_segments(&[], duration).is_empty());
        assert!(chapter_segments(&test_chapters(), Duration::ZERO).is_empty());
    }

    #[test]
    fn captions_and_settings_toggle() {
        let (mut app, _) = OtipApp::new();
        assert!(app.show_subs);
        let _ = app.update(Message::ToggleCaptions);
        assert!(!app.show_subs);
        assert!(!app.settings_open);
        let _ = app.update(Message::ToggleSettings);
        assert!(app.settings_open);
    }

    #[test]
    fn ai_scan_progress_lifecycle() {
        // Idle: no bar. Progress updates show it, completion hides it again.
        let (mut app, _) = OtipApp::new();
        assert_eq!(app.scan_progress, None);
        let _ = app.update(Message::AiScanProgress(0.45, "Sending batch 1/4 to AI...".into()));
        assert_eq!(app.scan_progress, Some(0.45));
        assert_eq!(app.scan_status, "Sending batch 1/4 to AI...");
        // Out-of-range fractions clamp into 0.0..=1.0.
        let _ = app.update(Message::AiScanProgress(1.5, "done".into()));
        assert_eq!(app.scan_progress, Some(1.0));
        let _ = app.update(Message::AiScanComplete(vec![]));
        assert_eq!(app.scan_progress, None);
        assert!(app.scan_status.is_empty());
    }

    #[test]
    fn quality_selection_applies() {
        use video_player::RenderQuality;
        let (mut app, _) = OtipApp::new();
        assert_eq!(app.render_quality, RenderQuality::P360);
        let _ = app.update(Message::QualitySelected(RenderQuality::P720));
        assert_eq!(app.render_quality, RenderQuality::P720);
        assert_eq!(RenderQuality::P720.dimensions(), (1280, 720));
        assert_eq!(RenderQuality::P540.label(), "540p");
    }

    fn test_sub_tracks() -> Vec<SubTrackInfo> {
        vec![
            SubTrackInfo { id: 1, title: "English".into(), lang: "en".into() },
            SubTrackInfo { id: 2, title: "Français".into(), lang: "fr".into() },
        ]
    }

    #[test]
    fn subtitle_options_list_tracks_with_off_first() {
        let options = build_subtitle_options(&test_sub_tracks());
        assert_eq!(options.len(), 3);
        assert_eq!(options[0], SubtitleOption::Off);
        assert_eq!(
            options[1],
            SubtitleOption::Track { id: 1, label: "English [en]".into() }
        );
        assert_eq!(options[2].to_string(), "Français [fr]");
        // No tracks: picker is just Off.
        assert_eq!(build_subtitle_options(&[]), vec![SubtitleOption::Off]);
    }

    #[test]
    fn subtitle_selection_drives_cc_state() {
        let (mut app, _) = OtipApp::new();
        let _ = app.update(Message::SubTracksLoaded(test_sub_tracks()));
        // Captions on by default: first track auto-selected (mpv behavior).
        assert_eq!(
            app.selected_subtitle,
            SubtitleOption::Track { id: 1, label: "English [en]".into() }
        );
        // Picking Off disables captions.
        let _ = app.update(Message::SubtitleSelected(SubtitleOption::Off));
        assert!(!app.show_subs);
        assert_eq!(app.selected_subtitle, SubtitleOption::Off);
        // CC toggle back on re-selects the first track.
        let _ = app.update(Message::ToggleCaptions);
        assert!(app.show_subs);
        assert_eq!(
            app.selected_subtitle,
            SubtitleOption::Track { id: 1, label: "English [en]".into() }
        );
    }

    #[test]
    fn palette_matches_spec_hex() {
        // Guard the YouTube/Spotify palette: main #121212, elevated #212121,
        // secondary #303030, text #FFFFFF/#AAAAAA, accent #3EA6FF, alert #CF6679.
        fn hex(c: Color) -> (u8, u8, u8) {
            (
                (c.r * 255.0).round() as u8,
                (c.g * 255.0).round() as u8,
                (c.b * 255.0).round() as u8,
            )
        }
        assert_eq!(hex(palette::BG_MAIN), (0x12, 0x12, 0x12));
        assert_eq!(hex(palette::BG_ELEVATED), (0x21, 0x21, 0x21));
        assert_eq!(hex(palette::BG_HOVER), (0x30, 0x30, 0x30));
        assert_eq!(hex(palette::TEXT_MAIN), (0xFF, 0xFF, 0xFF));
        assert_eq!(hex(palette::TEXT_DIM), (0xAA, 0xAA, 0xAA));
        assert_eq!(hex(palette::ACCENT), (0x3E, 0xA6, 0xFF));
        assert_eq!(hex(palette::ALERT), (0xCF, 0x66, 0x79));
    }

    #[test]
    fn mode_menu_opens_and_closes_on_select() {
        let (mut app, _) = OtipApp::new();
        assert!(!app.mode_menu_open);
        let _ = app.update(Message::ToggleModeMenu);
        assert!(app.mode_menu_open);
        let _ = app.update(Message::SetPlaybackMode(PlaybackMode::AutoSkip));
        assert_eq!(app.playback_mode, PlaybackMode::AutoSkip);
        assert!(!app.mode_menu_open);
    }

    #[test]
    fn stream_url_validation() {
        assert!(is_playable_url("https://example.com/video.mp4"));
        assert!(is_playable_url("http://example.com/live.m3u8"));
        assert!(is_playable_url("  https://example.com/v.mpd  "));
        assert!(!is_playable_url(""));
        assert!(!is_playable_url("ftp://example.com/video.mp4"));
        assert!(!is_playable_url("/home/user/video.mp4"));
        assert!(!is_playable_url("https://x"));
    }

    #[test]
    fn stream_display_name_uses_host() {
        assert_eq!(
            stream_display_name("https://cdn.example.com/live/playlist.m3u8"),
            "cdn.example.com/live/playlist.m3u8"
        );
        assert_eq!(stream_display_name("http://example.com"), "example.com");
        let long = format!("https://example.com/{}", "a".repeat(100));
        let shown = stream_display_name(&long);
        assert!(shown.ends_with('…'));
        assert_eq!(shown.chars().count(), 49);
    }

    #[test]
    fn url_dialog_flow_rejects_bad_input() {
        let (mut app, _) = OtipApp::new();
        let _ = app.update(Message::OpenUrlDialog);
        assert!(app.url_dialog_open);
        let _ = app.update(Message::UrlInputChanged("not a url".into()));
        let _ = app.update(Message::PlayUrl);
        // Rejected: stays in Library with an error status, no player spawned.
        assert_eq!(app.screen, AppScreen::Splash);
        assert!(app.video_player.is_none());
        assert!(app.status_is_error);
        let _ = app.update(Message::CloseUrlDialog);
        assert!(!app.url_dialog_open);
    }

    #[test]
    fn play_url_starts_stream_player() {
        let (mut app, _) = OtipApp::new();
        let _ = app.update(Message::OpenUrlDialog);
        let _ = app.update(Message::UrlInputChanged(
            "https://example.com/stream.m3u8".into(),
        ));
        let _ = app.update(Message::PlayUrl);
        assert_eq!(app.screen, AppScreen::Player);
        assert!(app.video_player.is_some());
        assert!(app.selected_video_path.is_none());
        assert_eq!(
            app.stream_title,
            Some("example.com/stream.m3u8".to_string())
        );
        assert!(!app.url_dialog_open);
        assert!(app.title().contains("example.com"));
        // No explicit stop: dropping `app` disconnects the command channel
        // and the render thread exits on its own. (CloseWindow needs a tokio
        // runtime for its detached stop task, so it can't run in unit tests.)
    }

    #[test]
    fn buffering_state_flips() {
        let (mut app, _) = OtipApp::new();
        assert!(!app.is_buffering);
        let _ = app.update(Message::Buffering(true));
        assert!(app.is_buffering);
        let _ = app.update(Message::Buffering(false));
        assert!(!app.is_buffering);
    }

    #[test]
    fn search_query_filters_videos() {
        let (mut app, _) = OtipApp::new();
        assert_eq!(app.search_query, "");
        let _ = app.update(Message::SearchQueryChanged("matrix".into()));
        assert_eq!(app.search_query, "matrix");
    }

    #[test]
    fn preplay_dialog_flow() {
        let (mut app, _) = OtipApp::new();
        assert!(!app.preplay_dialog_open);
        assert!(app.preplay_target.is_none());

        let video = PathBuf::from("/tmp/video.mp4");
        let _ = app.update(Message::OpenPreplayDialog(video.clone()));
        assert!(app.preplay_dialog_open);
        assert_eq!(app.preplay_target, Some(video));

        let _ = app.update(Message::ClosePreplayDialog);
        assert!(!app.preplay_dialog_open);
        assert!(app.preplay_target.is_none());
    }

    #[test]
    fn select_video_with_mode_applies() {
        let (mut app, _) = OtipApp::new();
        let video = PathBuf::from("/tmp/action_movie.mp4");
        let _ = app.update(Message::SelectVideoWithMode(video.clone(), PlaybackMode::InstantPlay));
        assert_eq!(app.playback_mode, PlaybackMode::InstantPlay);
        assert_eq!(app.screen, AppScreen::Player);
        assert_eq!(app.selected_video_path, Some(video));
        assert!(!app.preplay_dialog_open);
    }
}
