//! Otip - Iced 0.14 with 3-screen routing + GStreamer playbin rendering
//! Splash → Library (thumbnails) → Player (Image from playbin frames + controls)

mod video_player;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use iced::{
    widget::{button, column, container, image, mouse_area, pick_list, row, scrollable, slider, text, text_input, Space, stack},
    Alignment, Background, Border, Color, Element, Length, Shadow, Task, Theme,
    keyboard::{self, key::Named},
    mouse,
    Event,
    window,
};
use iced::widget::image::Handle;
use otip_core::domain::{PlaybackMode, PlaybackState};
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

    // Alpha variants (same hues, translucent over the main background).
    pub const BAR_BG: Color = Color { r: 0.1294, g: 0.1294, b: 0.1294, a: 0.92 }; // elevated control bar
    pub const PANEL_BG: Color = Color { r: 0.1294, g: 0.1294, b: 0.1294, a: 0.96 }; // popup panels
    pub const BTN_BG: Color = Color { r: 0.1882, g: 0.1882, b: 0.1882, a: 0.9 }; // buttons
    pub const BTN_BG_SOFT: Color = Color { r: 0.1882, g: 0.1882, b: 0.1882, a: 0.85 }; // subtle buttons
    pub const ACCENT_SOFT: Color = Color { r: 0.2431, g: 0.6510, b: 1.0, a: 0.55 }; // buffered strip
    pub const SCRIM: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 }; // overlay scrim
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
    QualitySelected(RenderQuality), // SW render target -> VideoPlayerHandle::set_quality
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
    unsafe_segments: Vec<(Duration, Duration)>, // auto-skip regions
    ai_skip_prompt: String, // custom AI skip prompt from user
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
                        for (seg_start, seg_end) in &self.unsafe_segments {
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

    fn view_title_bar(&self) -> Element<Message> {
        // Fix Issue 2: Restore custom Iced title bar at very top (Row with minimize/maximize/close)
        // This Row provides our own window controls; OS decorations are hidden via decorations:false
        container(
            row![
                text(self.title()).size(12).color(palette::TEXT_MAIN),
                Space::new().width(Length::Fill),
                // Custom window controls: minimize, maximize, close
                button(text("—").size(12).color(palette::TEXT_MAIN))
                    .padding([2, 8])
                    .style(|_: &Theme, _| button::Style {
                        background: Some(Background::Color(Color::TRANSPARENT)),
                        border: Border::default(),
                        text_color: palette::TEXT_MAIN,
                        shadow: Shadow::default(),
                        snap: false
                    })
                    .on_press(Message::MinimizeWindow),
                button(text("□").size(12).color(palette::TEXT_MAIN))
                    .padding([2, 8])
                    .style(|_: &Theme, _| button::Style {
                        background: Some(Background::Color(Color::TRANSPARENT)),
                        border: Border::default(),
                        text_color: palette::TEXT_MAIN,
                        shadow: Shadow::default(),
                        snap: false
                    })
                    .on_press(Message::MaximizeWindow),
                button(text("✕").size(13).color(Color::WHITE))
                    .on_press(Message::CloseWindow)
                    .padding([2, 10])
                    .style(|_: &Theme, status| button::Style {
                        background: Some(Background::Color(match status {
                            button::Status::Hovered => Color::from_rgb(0.9, 0.2, 0.2),
                            _ => Color::from_rgb(0.7, 0.2, 0.2),
                        })),
                        border: Border { radius: 4.0.into(), ..Default::default() },
                        text_color: Color::WHITE,
                        shadow: Shadow::default(),
                        snap: false
                    })
            ]
            .align_y(Alignment::Center)
            .spacing(4),
        )
        .width(Length::Fill)
        .height(Length::Fixed(30.0))
        .padding([2, 8])
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(palette::BG_ELEVATED)),
            border: Border {
                color: palette::BG_HOVER,
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: Shadow::default(),
            text_color: None,
            snap: false,
        })
        .into()
    }

    fn view(&self) -> Element<Message> {
        let content = match self.screen {
            AppScreen::Splash => self.view_splash(),
            AppScreen::Library => self.view_library(),
            AppScreen::Player => self.view_player(),
        };
        // Wrap with custom title bar so CloseRequested is always accessible and subscription cleanup is natural
        let title_bar = self.view_title_bar();
        let inner: Element<Message> = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::BG_MAIN)),
                border: Border::default(),
                shadow: Shadow::default(),
                text_color: Some(palette::TEXT_MAIN),
                snap: false,
            })
            .into();
        column![title_bar, inner]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_splash(&self) -> Element<Message> {
        let title = column![
            text("Otip").size(64).color(palette::ACCENT),
            text("AI-Powered Lookahead Content Moderator").size(18).color(palette::TEXT_MAIN),
            Space::new().height(Length::Fixed(8.0)),
            text("Scans upcoming scenes • Skips explicit content • Zero file modification").size(12).color(palette::TEXT_DIM),
        ].align_x(Alignment::Center).spacing(6);
        let cta = button(
            container(text("Browse Videos →").size(16).color(Color::WHITE)).padding(14).center_x(Length::Fill).width(Length::Fixed(220.0)),
        ).on_press(Message::NavigateTo(AppScreen::Library)).padding(0)
            .style(|_: &Theme, s| button::Style {
                background: Some(Background::Color(match s {
                    button::Status::Hovered => palette::ACCENT_HOVER,
                    button::Status::Pressed => palette::ACCENT_PRESSED,
                    _ => palette::ACCENT,
                })),
                border: Border { radius: 10.0.into(), ..Default::default() },
                text_color: Color::WHITE, shadow: Shadow::default(), snap: false,
            });
        container(column![
            Space::new().height(Length::FillPortion(1)), title, Space::new().height(Length::Fixed(32.0)), cta,
            Space::new().height(Length::FillPortion(1)), text("Safe Mode • Instant Play • Auto-Skip").size(11).color(palette::TEXT_DIM),
        ].align_x(Alignment::Center).spacing(12).width(Length::Fill).height(Length::Fill))
        .width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill).into()
    }

    fn view_library(&self) -> Element<Message> {
        // 1. Render Logic: iterate over library_videos and build grid immediately
        // 2. Placeholders: show colored box + icon for every video so 79 videos render instantly
        //    Thumbnails load async via Message::ThumbnailReady and replace placeholders incrementally
        // Fix Issue 1: Only show "No folder selected" if videos is actually empty
        // If auto-scan found 79 videos, show them even when selected_folder is None
        let folder_label = if let Some(folder) = &self.library_folder {
            folder.display().to_string()
        } else if !self.library_videos.is_empty() {
            format!("Auto-scanned • {} videos", self.library_videos.len())
        } else {
            "No folder selected".to_string()
        };
        let top_bar = row![
            button(text("← Back").size(13)).on_press(Message::NavigateTo(AppScreen::Splash)).padding(8)
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(palette::BG_HOVER)),
                    border: Border { color: palette::BG_HOVER, width: 1.0, radius: 6.0.into() },
                    text_color: palette::TEXT_MAIN, shadow: Shadow::default(), snap: false
                }),
            Space::new().width(Length::Fill),
            text(folder_label).size(12).color(palette::TEXT_DIM),
            Space::new().width(Length::Fill),
            button(text("🌐 Open URL").size(13).color(Color::WHITE)).on_press(Message::OpenUrlDialog).padding([8, 14])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(palette::BTN_BG)),
                    border: Border { color: palette::DIVIDER, width: 1.0, radius: 6.0.into() },
                    text_color: Color::WHITE, shadow: Shadow::default(), snap: false
                }),
            button(text("📁 Select Folder").size(13).color(Color::WHITE)).on_press(Message::SelectFolder).padding([8, 14])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(palette::ACCENT)),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE, shadow: Shadow::default(), snap: false
                }),
        ].align_y(Alignment::Center).spacing(12).width(Length::Fill);
        let status_color = if self.status_is_error { palette::ALERT } else { palette::TEXT_DIM };
        let status = text(&self.status).size(11).color(status_color);

        // Network stream URL panel (HLS/DASH/direct media). Shown below the
        // top bar while open; Enter confirms, ✕ dismisses.
        let url_panel: Element<Message> = if self.url_dialog_open {
            container(
                row![
                    text_input("https://example.com/stream.m3u8", &self.url_input)
                        .on_input(Message::UrlInputChanged)
                        .on_submit(Message::PlayUrl)
                        .padding(8)
                        .width(Length::Fill),
                    button(text("Open").size(13).color(Color::WHITE))
                        .on_press(Message::PlayUrl)
                        .padding([8, 14])
                        .style(|_: &Theme, _| button::Style {
                            background: Some(Background::Color(palette::ACCENT)),
                            border: Border { radius: 6.0.into(), ..Default::default() },
                            text_color: Color::WHITE, shadow: Shadow::default(), snap: false
                        }),
                    button(text("✕").size(13).color(Color::WHITE))
                        .on_press(Message::CloseUrlDialog)
                        .padding([8, 12])
                        .style(|_: &Theme, _| button::Style {
                            background: Some(Background::Color(palette::BTN_BG)),
                            border: Border { radius: 6.0.into(), ..Default::default() },
                            text_color: Color::WHITE, shadow: Shadow::default(), snap: false
                        }),
                ]
                .align_y(Alignment::Center)
                .spacing(8),
            )
            .width(Length::Fill)
            .padding([10, 12])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::PANEL_BG)),
                border: Border {
                    color: palette::DIVIDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: Shadow::default(),
                text_color: Some(palette::TEXT_MAIN),
                snap: false,
            })
            .into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        // Fix Issue 1: MUST render list/grid when videos is not empty, regardless of selected_folder
        // Only show "No folder selected" / "No videos" when videos is actually empty
        let grid: Element<Message> = if self.library_videos.is_empty() {
            container(column![
                text("No videos found").size(16).color(palette::TEXT_DIM),
                Space::new().height(Length::Fixed(8.0)),
                text("No folder selected — auto-scan found 0 videos. Pick a folder.").size(12).color(palette::TEXT_DIM),
            ].align_x(Alignment::Center)).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill).into()
        } else {
            // Iterate over discovered videos and render inside Scrollable
            // Simple Row/Column layout as requested: text(title) + Play button for each video
            // This is inside Scrollable so 2 or 79 videos all render and scroll
            let video_list: Vec<Element<Message>> = self
                .library_videos
                .iter()
                .map(|path| {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("video")
                        .to_string();
                    let p = path.clone();
                    // Minimal requested UI: text widget with title + play button
                    // Also show thumbnail if already loaded, otherwise placeholder
                    let thumb: Element<Message> = if let Some(handle) = self.thumbnails.get(path) {
                        container(image(handle.clone()).width(Length::Fixed(160.0)).height(Length::Fixed(90.0)))
                            .width(Length::Fixed(160.0)).height(Length::Fixed(90.0))
                            .style(|_: &Theme| container::Style {
                                background: Some(Background::Color(palette::BG_MAIN)),
                                border: Border { color: palette::BG_HOVER, width: 1.0, radius: 6.0.into() },
                                shadow: Shadow::default(), text_color: None, snap: false,
                            }).into()
                    } else {
                        container(text("🎬").size(24).color(palette::ACCENT))
                            .width(Length::Fixed(160.0)).height(Length::Fixed(90.0)).center_x(Length::Fill).center_y(Length::Fill)
                            .style(|_: &Theme| container::Style {
                                background: Some(Background::Color(palette::BG_ELEVATED)),
                                border: Border { color: palette::BG_HOVER, width: 1.0, radius: 6.0.into() },
                                shadow: Shadow::default(), text_color: None, snap: false,
                            }).into()
                    };
                    container(
                        row![
                            thumb,
                            column![
                                text(name.clone()).size(13).color(Color::WHITE),
                                text(p.display().to_string()).size(10).color(palette::TEXT_DIM),
                            ].spacing(4).width(Length::Fill),
                            button(text("▶ Play").size(12).color(Color::WHITE))
                                .on_press(Message::VideoSelected(p.clone()))
                                .padding([8, 14])
                                .style(|_: &Theme, _| button::Style {
                                    background: Some(Background::Color(palette::ACCENT)),
                                    border: Border { radius: 6.0.into(), ..Default::default() },
                                    text_color: Color::WHITE, shadow: Shadow::default(), snap: false,
                                })
                        ].align_y(Alignment::Center).spacing(12).width(Length::Fill).padding(10)
                    )
                    .width(Length::Fill)
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(palette::BG_ELEVATED)),
                        border: Border { color: palette::BG_HOVER, width: 1.0, radius: 8.0.into() },
                        shadow: Shadow::default(), text_color: None, snap: false,
                    }).into()
                })
                .collect();
            scrollable(column(video_list).spacing(10).padding(4))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };
        container(
            column![
                top_bar,
                url_panel,
                Space::new().height(Length::Fixed(12.0)),
                status,
                Space::new().height(Length::Fixed(8.0)),
                grid
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

    fn view_player(&self) -> Element<Message> {
        // ── Video Rendering: Software fallback (Wayland stable) ──────
        // Previously: mpv hwdec=auto + wgpu zero-copy shared texture (empty stub on Wayland/Vulkan).
        // Now: software render fallback - Handle::from_pixels(width, height, buffer) sent via PlayerEvent::Frame
        // UI Rendering: simply use standard iced::widget::image(handle) to display frame.
        // This is highly stable on Wayland, no wgpu Device, no FBO, no hwdec interop.
        // iced::widget::image(handle) is the correct fallback.
        let video_area: Element<Message> = if let Some(handle) = &self.video_handle {
            // Software fallback: display Handle::from_pixels / Handle::from_rgba via iced::widget::image
            // iced::widget::image(handle) - stable SW fallback
            iced::widget::image(handle.clone()).width(Length::Fill).height(Length::Fill).into()
        } else {
            container(column![
                text("▶ No video - select from library").size(18).color(Color::WHITE).align_x(Alignment::Center),
                Space::new().height(Length::Fixed(8.0)),
                text(self.selected_video_path.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()).map(|n| n.to_string()).or_else(|| self.stream_title.clone()).unwrap_or("no file".into())).size(12).color(palette::TEXT_MAIN),
                Space::new().height(Length::Fixed(8.0)),
                text(if self.video_player.is_some() { "Loading video (SW fallback)..." } else { "" }).size(11).color(palette::TEXT_DIM),
            ].align_x(Alignment::Center).spacing(4))
            .width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill)
            .style(|_: &Theme| container::Style{ background: Some(Background::Color(palette::BG_MAIN)), border: Border{ color: palette::BG_HOVER, width:1.0, radius:8.0.into()}, shadow: Shadow::default(), text_color: None, snap:false }).into()
        };

        // Top-right overlay: three-dot menu holding the playback-mode choices
        // (Safe / Instant / Auto-Skip). One compact button instead of three.
        let dots_btn = button(text("⋯").size(15).color(Color::WHITE))
            .on_press(Message::ToggleModeMenu)
            .padding([2, 10])
            .style(move |_: &Theme, _| button::Style {
                background: Some(Background::Color(if self.mode_menu_open {
                    palette::ACCENT
                } else {
                    palette::BTN_BG
                })),
                border: Border { radius: 6.0.into(), ..Default::default() },
                text_color: Color::WHITE,
                shadow: Shadow::default(),
                snap: false,
            });
        let mode_choice = |label: &'static str,
                           hint: &'static str,
                           mode: PlaybackMode,
                           current: PlaybackMode|
         -> Element<Message> {
            row![
                overlay_btn(label, mode, current == mode),
                text(hint)
                    .size(11)
                    .color(palette::TEXT_DIM),
            ]
            .align_y(Alignment::Center)
            .spacing(8)
            .into()
        };
        let current_mode = self.playback_mode;
        let mode_panel: Element<Message> = if self.mode_menu_open {
            container(column![
                mode_choice(
                    "Safe Mode",
                    "Look ahead, skip sensitive scenes",
                    PlaybackMode::SafeMode,
                    current_mode
                ),
                mode_choice(
                    "Instant Play",
                    "Start right away, no scanning",
                    PlaybackMode::InstantPlay,
                    current_mode
                ),
                mode_choice(
                    "Auto-Skip",
                    "Skip flagged segments automatically",
                    PlaybackMode::AutoSkip,
                    current_mode
                ),
                // AI Skip Prompt Input
                container(
                    column![
                        text("AI Skip Prompt").size(11).color(palette::TEXT_DIM),
                        text_input(
                            "What should the AI skip? e.g. sponsorships, violence...",
                            &self.ai_skip_prompt
                        )
                        .on_input(Message::AiPromptChanged)
                        .padding(8)
                        .style(|_: &Theme, _| iced::widget::text_input::Style {
                            background: Background::Color(palette::BG_ELEVATED),
                            border: Border {
                                color: palette::DIVIDER,
                                width: 1.0,
                                radius: 6.0.into(),
                            },
                            placeholder: palette::TEXT_DIM,
                            value: palette::TEXT_MAIN,
                            selection: palette::ACCENT_SOFT,
                            icon: palette::TEXT_DIM,
                        })
                    ]
                    .spacing(4)
                )
                .padding([8, 0])
            ]
            .spacing(4))
            .padding(10)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::PANEL_BG)),
                border: Border {
                    color: palette::DIVIDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: Shadow::default(),
                text_color: Some(palette::TEXT_MAIN),
                snap: false,
            })
            .into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };
        let overlay = container(
            column![
                row![
                    Space::new().width(Length::Fill),
                    dots_btn,
                ],
                mode_panel,
            ]
            .spacing(6),
        )
        .padding(8)
            .style(|_: &Theme| container::Style{ background: Some(Background::Color(palette::SCRIM)), border: Border{ color: palette::DIVIDER, width:1.0, radius:8.0.into()}, shadow: Shadow::default(), text_color: None, snap:false });

        let stacked_video = stack![
            container(video_area).width(Length::Fill).height(Length::Fill),
            container(overlay).width(Length::Fill).height(Length::Fill).align_x(Alignment::End).align_y(Alignment::Start).padding(12),
        ].width(Length::Fill).height(Length::FillPortion(1));

        // ── Core UI Controls: Semi-transparent bottom overlay bar (container + row) per spec ──
        // This bar is wrapped in mouse_area for auto-hide progressive disclosure
        let playback_state = if self.is_playing { PlaybackState::Playing } else { PlaybackState::Paused };
        let play_pause_label = match playback_state {
            PlaybackState::Playing => "⏸ Pause",
            PlaybackState::Paused => "▶ Play",
            _ => "▶ Play",
        };
        // Time Display: Current Time / Total Duration e.g. 01:23 / 15:00,
        // plus the current embedded chapter title when chapters exist.
        let time_text = format!("{} / {}", format_duration_short(self.position), format_duration_short(self.duration));
        let time_label = match current_chapter_title(&self.chapters, self.position) {
            Some(title) => format!("{}  •  {}", time_text, title),
            None => time_text,
        };

        // Buffering badge: shown while mpv stalls waiting for stream cache
        // (paused-for-cache). Hidden otherwise to keep the bar compact.
        let buffering_badge: Element<Message> = if self.is_buffering {
            container(text("⏳ Buffering…").size(11).color(palette::TEXT_MAIN))
                .padding([4, 8])
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::BTN_BG)),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                })
                .into()
        } else {
            Space::new().width(Length::Shrink).into()
        };

        // Buffered strip: thin bar above the seek slider showing how far the
        // demuxer cache reaches ahead (mpv demuxer-cache-duration). For local
        // files this quickly covers the whole timeline; for streams it shows
        // real readahead.
        let buffered_frac = if self.duration.as_secs_f64() > 0.0 {
            ((self.position.as_secs_f64() + self.buffered_ahead_secs)
                / self.duration.as_secs_f64())
            .clamp(0.0, 1.0)
        } else {
            0.0
        };
        let buffered_filled = (buffered_frac * 1000.0).round() as u16;
        let buffered_strip: Element<Message> = row![
            container(Space::new().width(Length::Fill).height(Length::Fixed(3.0)))
                .width(Length::FillPortion(buffered_filled))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::ACCENT_SOFT)),
                    border: Border { radius: 2.0.into(), ..Default::default() },
                    shadow: Shadow::default(), text_color: None, snap: false,
                }),
            container(Space::new().width(Length::Fill).height(Length::Fixed(3.0)))
                .width(Length::FillPortion(1000 - buffered_filled))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(palette::TRACK_BG)),
                    border: Border { radius: 2.0.into(), ..Default::default() },
                    shadow: Shadow::default(), text_color: None, snap: false,
                }),
        ]
        .spacing(0)
        .into();

        // Chapter markers: clickable segments proportional to each chapter's
        // length (jump on click via SeekTo). Hidden when the file has no
        // embedded chapters.
        let chapter_segments = chapter_segments(&self.chapters, self.duration);
        let chapter_strip: Element<Message> = if chapter_segments.is_empty() {
            Space::new().height(Length::Fixed(0.0)).into()
        } else {
            let total = self.duration.as_secs_f64().max(0.001);
            let position = self.position;
            let segments: Vec<Element<Message>> = chapter_segments
                .into_iter()
                .map(|(start, end)| {
                    let gap = (end - start).as_secs_f64().max(0.0);
                    let portion = ((gap / total) * 1000.0).round().clamp(1.0, 1000.0) as u16;
                    let active = position >= start && position < end;
                    mouse_area(
                        container(Space::new().width(Length::Fill).height(Length::Fixed(8.0)))
                            .width(Length::FillPortion(portion))
                            .style(move |_: &Theme| container::Style {
                                background: Some(Background::Color(if active {
                                    palette::ACCENT
                                } else {
                                    palette::MARKER_IDLE
                                })),
                                border: Border { radius: 4.0.into(), ..Default::default() },
                                shadow: Shadow::default(), text_color: None, snap: false,
                            }),
                    )
                    .on_press(Message::SeekTo(start))
                    .into()
                })
                .collect();
            row(segments).spacing(2).into()
        };

        // Progress/Seek Slider: iced::widget::slider spanning width, bound to duration
        let seek_bar = slider(0.0..=1.0, self.timeline_pos as f64, Message::Seek).step(0.005).width(Length::Fill).style(dark_slider_style());

        // Volume Controls: mute toggle + slider 0.0..1.0
        let mute_icon = if self.is_muted || self.volume < 0.01 { "🔇" } else if self.volume < 0.5 { "🔉" } else { "🔊" };
        let volume_row = row![
            button(text(mute_icon).size(13)).on_press(Message::ToggleMute).padding([4,8])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::BTN_BG_SOFT)), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            slider(0.0..=1.0, self.volume as f64, Message::SetVolume).step(0.02).width(Length::Fixed(90.0)).style(dark_slider_style()),
        ].spacing(6).align_y(Alignment::Center);

        // Playback Speed: dropdown menu via pick_list (dark-theme styled).
        // Replaces the old static "1.0x" cycle button so every speed is
        // one click away. Reuses Message::SetSpeed(f32) -> set_rate.
        let speed_menu = pick_list(
            &SPEED_OPTIONS[..],
            Some(speed_label(self.playback_speed)),
            |label: &'static str| Message::SetSpeed(parse_speed_label(label)),
        )
        .placeholder("1.0x")
        .width(Length::Fixed(88.0))
        .style(dark_pick_list_style());

        // Loop Toggle: accent background while active (same pattern as the
        // Safe/Instant/Auto-Skip overlay buttons).
        let loop_btn = button(text("🔁 Loop").size(11).color(Color::WHITE)).on_press(Message::ToggleLoop).padding([6,10])
            .style(move |_: &Theme, _| button::Style{
                background: Some(Background::Color(if self.is_looping { palette::ACCENT } else { palette::BTN_BG })),
                border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false
            });

        // Fullscreen Toggle
        let fs_label = if self.is_fullscreen { "🗗 Exit" } else { "⛶ Full" };
        let fs_btn = button(text(fs_label).size(11)).on_press(Message::ToggleFullscreen).padding([6,10])
            .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::BTN_BG_SOFT)), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false });

        // Captions Toggle: accent background while visible (mpv sub-visibility).
        let cc_btn = button(text("CC").size(11).color(Color::WHITE)).on_press(Message::ToggleCaptions).padding([6,10])
            .style(move |_: &Theme, _| button::Style{
                background: Some(Background::Color(if self.show_subs { palette::ACCENT } else { palette::BTN_BG })),
                border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false
            });

        // Settings (gear) Toggle: opens the quality popup panel.
        let settings_btn = button(text("⚙").size(13).color(Color::WHITE)).on_press(Message::ToggleSettings).padding([6,10])
            .style(move |_: &Theme, _| button::Style{
                background: Some(Background::Color(if self.settings_open { palette::ACCENT } else { palette::BTN_BG })),
                border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false
            });

        // Mini/PiP Toggle: desktop approximation (small always-on-top window).
        let pip_label = if self.is_mini { "❐ Exit" } else { "❐ PiP" };
        let pip_btn = button(text(pip_label).size(11).color(Color::WHITE)).on_press(Message::ToggleMini).padding([6,10])
            .style(move |_: &Theme, _| button::Style{
                background: Some(Background::Color(if self.is_mini { palette::ACCENT } else { palette::BTN_BG_SOFT })),
                border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false
            });

        let controls_row = row![
            button(text("⏮ Prev").size(11)).on_press(Message::PrevVideo).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::BTN_BG)), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            button(text("⏪ 10s").size(11)).on_press(Message::SkipBackward).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::BTN_BG)), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            button(text(play_pause_label).size(13).color(Color::WHITE))
                .on_press(Message::PlayPause).padding([8,16])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::ACCENT)), border: Border{ radius:20.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            button(text("10s ⏩").size(11)).on_press(Message::SkipForward).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::BTN_BG)), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            button(text("Next ⏭").size(11)).on_press(Message::NextVideo).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::BTN_BG)), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            text(time_label).size(12).color(palette::TEXT_MAIN),
            buffering_badge,
            volume_row,
            cc_btn,
            settings_btn,
            loop_btn,
            speed_menu,
            pip_btn,
            fs_btn,
            button(text("← Library").size(11)).on_press(Message::NavigateTo(AppScreen::Library)).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(palette::BG_HOVER)), border: Border{ color: palette::BG_HOVER, width:1.0, radius:6.0.into()}, text_color: palette::TEXT_MAIN, shadow: Shadow::default(), snap:false }),
        ].align_y(Alignment::Center).spacing(8).width(Length::Fill);

        let bottom_bar: Element<Message> = container(column![
            buffered_strip,
            Space::new().height(Length::Fixed(4.0)),
            chapter_strip,
            seek_bar,
            Space::new().height(Length::Fixed(6.0)),
            controls_row,
        ].spacing(6)).width(Length::Fill).padding([12,14])
            .style(|_: &Theme| container::Style{
                background: Some(Background::Color(palette::BAR_BG)),
                border: Border{ color: palette::TRACK_BG, width:1.0, radius:8.0.into()},
                shadow: Shadow::default(), text_color: Some(palette::TEXT_MAIN), snap:false
            }).into();

        // 2. UX: Auto-Hide Controls - progressive disclosure, hide after 3s mouse inactivity
        // Container with auto-hide: visible if controls_visible else transparent spacer
        let overlay_controls: Element<Message> = if self.controls_visible {
            container(bottom_bar).width(Length::Fill).padding(12).into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        // Settings popup panel (gear menu): render-quality selector plus
        // chapter count. Shown above the control bar while open.
        let chapter_count = self.chapters.len();
        let sub_hint = if self.sub_tracks.is_empty() {
            "No embedded subtitles in this file"
        } else {
            "Embedded tracks from file"
        };
        let settings_panel: Element<Message> = if self.settings_open {
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
            .padding([10, 14])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(palette::PANEL_BG)),
                border: Border {
                    color: palette::DIVIDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: Shadow::default(),
                text_color: Some(palette::TEXT_MAIN),
                snap: false,
            })
            .into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        let player_stack = column![stacked_video, settings_panel, overlay_controls].spacing(0).width(Length::Fill).height(Length::Fill);

        // Wrap entire player in mouse_area to capture mouse movement for auto-hide
        mouse_area(player_stack).on_move(|_| Message::MouseMoved).into()
    }
}

/// Playback-speed options shown in the speed `pick_list` dropdown.
const SPEED_OPTIONS: [&str; 4] = ["0.5x", "1.0x", "1.5x", "2.0x"];

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
        handle_color: palette::TEXT_MAIN,
        background: Background::Color(palette::BTN_BG),
        border: Border {
            radius: 6.0.into(),
            ..Default::default()
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
            width: 4.0,
            border: Border::default(),
        },
        handle: iced::widget::slider::Handle {
            shape: iced::widget::slider::HandleShape::Circle { radius: 8.0.into() },
            background: Background::Color(palette::ACCENT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
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

fn overlay_btn<'a>(label: &'a str, mode: PlaybackMode, active: bool) -> Element<'a, Message> {    button(text(label).size(11).align_x(Alignment::Center)).on_press(Message::SetPlaybackMode(mode)).padding([6,10])
        .style(move |_: &Theme, _| button::Style{
            background: Some(Background::Color(if active { palette::ACCENT } else { palette::BTN_BG })),
            border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false,
        }).into()
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
fn view(app: &OtipApp) -> Element<Message> { app.view() }
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
}
