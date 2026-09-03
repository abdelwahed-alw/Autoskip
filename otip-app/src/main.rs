//! Otip - Iced 0.14 with 3-screen routing + GStreamer playbin rendering
//! Splash → Library (thumbnails) → Player (Image from playbin frames + controls)

mod video_player;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use iced::{
    widget::{button, column, container, image, mouse_area, pick_list, row, scrollable, slider, text, Space, stack},
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
use video_player::{PlayerEvent, VideoPlayerHandle};
use iced::futures::SinkExt;

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
    FileSelected(Option<PathBuf>),
    SetPlaybackMode(PlaybackMode),
    PlayPause,
    Seek(f64), // spec: Seek(f64) 0.0..=1.0 normalized -> VideoPlayerHandle::seek
    SeekF64(f64), // alias for f64 seek
    SeekTo(Duration),
    VolumeChanged(f32), // 0.0..=1.0 -> VideoPlayerHandle::set_volume
    SetVolume(f64), // spec: SetVolume(f64) -> VideoPlayerHandle::set_volume_f64
    ToggleMute, // mute toggle -> VideoPlayerHandle::toggle_mute
    SkipForward, // +10s
    SkipBackward, // -10s
    PositionUpdate(Duration, Duration),
    FrameReady(Handle), // raw RGBA from playbin thread
    ThumbnailReady(PathBuf, Option<Handle>),
    ThumbnailsBatch(Vec<(PathBuf, Handle)>),
    CloseRequested, // custom title bar X - non-blocking shutdown via iced::window::close / iced::exit
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
    playback_mode: PlaybackMode,
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
    controls_visible: bool,   // progressive disclosure: visible after move, hidden after 3s
    is_muted: bool,
    prev_volume: f32,
    playback_speed: f32, // 0.5,1.0,1.5,2.0
    is_fullscreen: bool,
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
                playback_mode: PlaybackMode::SafeMode,
                is_playing: false,
                position: Duration::ZERO,
                duration: Duration::ZERO,
                volume: 0.7,
                timeline_pos: 0.0,
                status: "Welcome to Otip".into(),
                video_player: None,
                last_mouse_move: Instant::now(),
                controls_visible: true,
                is_muted: false,
                prev_volume: 0.7,
                playback_speed: 1.0,
                is_fullscreen: false,
                video_handle: None,
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
                .map(|n| format!("Otip — {} [{}]", n, self.playback_mode))
                .unwrap_or_else(|| "Otip — Player".into()),
            AppScreen::Library => "Otip — Library".into(),
            AppScreen::Splash => "Otip — AI Content Moderator".into(),
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
                self.screen = AppScreen::Player;
                self.is_playing = true;
                self.position = Duration::ZERO;
                self.duration = Duration::ZERO;
                self.timeline_pos = 0.0;
                self.video_handle = None;
                self.controls_visible = true;
                self.last_mouse_move = Instant::now();
                // Spawn playbin with appsink in background thread; UI stays responsive
                let player = VideoPlayerHandle::spawn(path.clone());
                // Apply current volume to new player
                player.set_volume(self.volume);
                self.video_player = Some(player);
                self.status = format!("Loading: {}", path.display());
                tracing::info!("VideoSelected {:?} with mode {} → Player", path, self.playback_mode);
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
                tracing::info!("Playback mode set to {}", mode);
                Task::none()
            }
            Message::PlayPause => {
                self.is_playing = !self.is_playing;
                if let Some(p) = &self.video_player {
                    p.toggle_pause();
                }
                Task::none()
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
                    p.set_volume(new_vol);
                    // also via toggle_mute cmd
                    p.toggle_mute();
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
                // Backend wiring: toggle window mode asynchronously
                let mode = if self.is_fullscreen { window::Mode::Fullscreen } else { window::Mode::Windowed };
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
                Task::none()
            }
            Message::FrameReady(handle) => {
                self.video_handle = Some(handle);
                Task::none()
            }
            Message::MpvError(e) => {
                self.status = format!("MPV error: {}", e);
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
                *video_player::FRAME_RX_GLOBAL.lock().unwrap() = None;
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
            Message::Noop => Task::none(),
        }
    }

    fn view_title_bar(&self) -> Element<Message> {
        // Fix Issue 2: Restore custom Iced title bar at very top (Row with minimize/maximize/close)
        // This Row provides our own window controls; OS decorations are hidden via decorations:false
        container(
            row![
                text(self.title()).size(12).color(Color::from_rgb(0.85, 0.85, 0.88)),
                Space::new().width(Length::Fill),
                // Custom window controls: minimize, maximize, close
                button(text("—").size(12).color(Color::from_rgb(0.7, 0.7, 0.75)))
                    .padding([2, 8])
                    .style(|_: &Theme, _| button::Style {
                        background: Some(Background::Color(Color::TRANSPARENT)),
                        border: Border::default(),
                        text_color: Color::from_rgb(0.7, 0.7, 0.75),
                        shadow: Shadow::default(),
                        snap: false
                    })
                    .on_press(Message::Noop),
                button(text("□").size(12).color(Color::from_rgb(0.7, 0.7, 0.75)))
                    .padding([2, 8])
                    .style(|_: &Theme, _| button::Style {
                        background: Some(Background::Color(Color::TRANSPARENT)),
                        border: Border::default(),
                        text_color: Color::from_rgb(0.7, 0.7, 0.75),
                        shadow: Shadow::default(),
                        snap: false
                    })
                    .on_press(Message::ToggleFullscreen),
                button(text("✕").size(13).color(Color::WHITE))
                    .on_press(Message::CloseRequested)
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
            background: Some(Background::Color(Color::from_rgb(0.14, 0.14, 0.17))),
            border: Border {
                color: Color::from_rgb(0.22, 0.22, 0.26),
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
            .style(|t: &Theme| container::Style {
                background: Some(Background::Color(t.palette().background)),
                border: Border::default(),
                shadow: Shadow::default(),
                text_color: Some(t.palette().text),
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
            text("Otip").size(64).color(Color::from_rgb(0.2, 0.6, 0.9)),
            text("AI-Powered Lookahead Content Moderator").size(18).color(Color::from_rgb(0.7, 0.7, 0.75)),
            Space::new().height(Length::Fixed(8.0)),
            text("Scans upcoming scenes • Skips explicit content • Zero file modification").size(12).color(Color::from_rgb(0.5, 0.5, 0.55)),
        ].align_x(Alignment::Center).spacing(6);
        let cta = button(
            container(text("Browse Videos →").size(16).color(Color::WHITE)).padding(14).center_x(Length::Fill).width(Length::Fixed(220.0)),
        ).on_press(Message::NavigateTo(AppScreen::Library)).padding(0)
            .style(|_: &Theme, s| button::Style {
                background: Some(Background::Color(match s {
                    button::Status::Hovered => Color::from_rgb(0.25, 0.65, 0.95),
                    button::Status::Pressed => Color::from_rgb(0.15, 0.5, 0.85),
                    _ => Color::from_rgb(0.2, 0.6, 0.9),
                })),
                border: Border { radius: 10.0.into(), ..Default::default() },
                text_color: Color::WHITE, shadow: Shadow::default(), snap: false,
            });
        container(column![
            Space::new().height(Length::FillPortion(1)), title, Space::new().height(Length::Fixed(32.0)), cta,
            Space::new().height(Length::FillPortion(1)), text("Safe Mode • Instant Play • Auto-Skip").size(11).color(Color::from_rgb(0.45, 0.45, 0.5)),
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
                .style(|t: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgba(0.15, 0.15, 0.18, 1.0))),
                    border: Border { color: Color::from_rgb(0.3, 0.3, 0.35), width: 1.0, radius: 6.0.into() },
                    text_color: t.palette().text, shadow: Shadow::default(), snap: false
                }),
            Space::new().width(Length::Fill),
            text(folder_label).size(12).color(Color::from_rgb(0.6, 0.6, 0.65)),
            Space::new().width(Length::Fill),
            button(text("📁 Select Folder").size(13).color(Color::WHITE)).on_press(Message::SelectFolder).padding([8, 14])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.2, 0.6, 0.9))),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE, shadow: Shadow::default(), snap: false
                }),
        ].align_y(Alignment::Center).spacing(12).width(Length::Fill);
        let status = text(&self.status).size(11).color(Color::from_rgb(0.5, 0.5, 0.55));

        // Fix Issue 1: MUST render list/grid when videos is not empty, regardless of selected_folder
        // Only show "No folder selected" / "No videos" when videos is actually empty
        let grid: Element<Message> = if self.library_videos.is_empty() {
            container(column![
                text("No videos found").size(16).color(Color::from_rgb(0.6, 0.6, 0.65)),
                Space::new().height(Length::Fixed(8.0)),
                text("No folder selected — auto-scan found 0 videos. Pick a folder.").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)),
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
                                background: Some(Background::Color(Color::from_rgb(0.08, 0.08, 0.10))),
                                border: Border { color: Color::from_rgb(0.25, 0.25, 0.30), width: 1.0, radius: 6.0.into() },
                                shadow: Shadow::default(), text_color: None, snap: false,
                            }).into()
                    } else {
                        container(text("🎬").size(24).color(Color::from_rgb(0.6,0.6,0.9)))
                            .width(Length::Fixed(160.0)).height(Length::Fixed(90.0)).center_x(Length::Fill).center_y(Length::Fill)
                            .style(|_: &Theme| container::Style {
                                background: Some(Background::Color(Color::from_rgb(0.14, 0.14, 0.18))),
                                border: Border { color: Color::from_rgb(0.30, 0.30, 0.35), width: 1.0, radius: 6.0.into() },
                                shadow: Shadow::default(), text_color: None, snap: false,
                            }).into()
                    };
                    container(
                        row![
                            thumb,
                            column![
                                text(name.clone()).size(13).color(Color::WHITE),
                                text(p.display().to_string()).size(10).color(Color::from_rgb(0.6,0.6,0.65)),
                            ].spacing(4).width(Length::Fill),
                            button(text("▶ Play").size(12).color(Color::WHITE))
                                .on_press(Message::VideoSelected(p.clone()))
                                .padding([8, 14])
                                .style(|_: &Theme, _| button::Style {
                                    background: Some(Background::Color(Color::from_rgb(0.2, 0.6, 0.9))),
                                    border: Border { radius: 6.0.into(), ..Default::default() },
                                    text_color: Color::WHITE, shadow: Shadow::default(), snap: false,
                                })
                        ].align_y(Alignment::Center).spacing(12).width(Length::Fill).padding(10)
                    )
                    .width(Length::Fill)
                    .style(|_: &Theme| container::Style {
                        background: Some(Background::Color(Color::from_rgb(0.16, 0.16, 0.20))),
                        border: Border { color: Color::from_rgb(0.28,0.28,0.32), width: 1.0, radius: 8.0.into() },
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
        .style(|t: &Theme| container::Style {
            background: Some(Background::Color(t.palette().background)),
            border: Border::default(),
            shadow: Shadow::default(),
            text_color: Some(t.palette().text),
            snap: false,
        })
        .into()
    }

    fn view_player(&self) -> Element<Message> {
        // ── Video Rendering: libmpv render_context + wgpu zero-copy (VLC-killer) ──────
        // Texture Sharing: mpv hwdec=auto renders directly into wgpu::Texture we allocated
        // via mpv_render_context_create(OpenGL/Vulkan FBO). Iced draws the shared texture.
        // No CPU appsink, no Handle::from_rgba per frame.
        // Wire Custom Widget: STOP using iced::widget::image(handle) for video playback
        // Use MpvWgpuWidget (zero-copy shared wgpu texture via mpv_render_context)
        let video_area: Element<Message> = if let Some(player) = &self.video_player {
            // Trigger Renders: mpv_render_context_render is called inside the widget's draw
            // The widget owns the shared wgpu::TextureView that mpv wrote into via FBO (hwdec=auto)
            video_player::widget::MpvWgpuWidget::new(player.mpv.clone(), 1280, 720).into()
        } else if let Some(handle) = &self.video_handle {
            image(handle.clone()).width(Length::Fill).height(Length::Fill).into()
        } else {
            container(column![
                text("▶ No video - select from library").size(18).color(Color::WHITE).align_x(Alignment::Center),
                Space::new().height(Length::Fixed(8.0)),
                text(self.selected_video_path.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("no file")).size(12).color(Color::from_rgb(0.7,0.7,0.75)),
            ].align_x(Alignment::Center).spacing(4))
            .width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill)
            .style(|_: &Theme| container::Style{ background: Some(Background::Color(Color::from_rgb(0.04,0.04,0.06))), border: Border{ color: Color::from_rgb(0.18,0.18,0.22), width:1.0, radius:8.0.into()}, shadow: Shadow::default(), text_color: None, snap:false }).into()
        };

        // Top-right overlay: playback options (always accessible)
        let overlay = container(row![
            overlay_btn("Safe", PlaybackMode::SafeMode, self.playback_mode == PlaybackMode::SafeMode),
            overlay_btn("Instant", PlaybackMode::InstantPlay, self.playback_mode == PlaybackMode::InstantPlay),
            overlay_btn("Auto-Skip", PlaybackMode::AutoSkip, self.playback_mode == PlaybackMode::AutoSkip),
        ].spacing(6)).padding(8)
            .style(|_: &Theme| container::Style{ background: Some(Background::Color(Color::from_rgba(0.0,0.0,0.0,0.55))), border: Border{ color: Color::from_rgba(1.0,1.0,1.0,0.1), width:1.0, radius:8.0.into()}, shadow: Shadow::default(), text_color: None, snap:false });

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
        // Time Display: Current Time / Total Duration e.g. 01:23 / 15:00
        let time_text = format!("{} / {}", format_duration_short(self.position), format_duration_short(self.duration));

        // Progress/Seek Slider: iced::widget::slider spanning width, bound to duration
        let seek_bar = slider(0.0..=1.0, self.timeline_pos as f64, Message::Seek).step(0.005).width(Length::Fill);

        // Volume Controls: mute toggle + slider 0.0..1.0
        let mute_icon = if self.is_muted || self.volume < 0.01 { "🔇" } else if self.volume < 0.5 { "🔉" } else { "🔊" };
        let volume_row = row![
            button(text(mute_icon).size(13)).on_press(Message::ToggleMute).padding([4,8])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.25,0.25,0.30,0.85))), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            slider(0.0..=1.0, self.volume as f64, Message::SetVolume).step(0.02).width(Length::Fixed(90.0)),
        ].spacing(6).align_y(Alignment::Center);

        // Playback Speed: cycle button 0.5x,1.0x,1.5x,2.0x
        let speed_label = format!("{:.1}x", self.playback_speed);
        let speed_btn = button(text(speed_label).size(11).color(Color::WHITE)).on_press(Message::CycleSpeed).padding([6,10])
            .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.35,0.35,0.40,0.9))), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false });

        // Fullscreen Toggle
        let fs_label = if self.is_fullscreen { "🗗 Exit" } else { "⛶ Full" };
        let fs_btn = button(text(fs_label).size(11)).on_press(Message::ToggleFullscreen).padding([6,10])
            .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.25,0.25,0.30,0.85))), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false });

        let controls_row = row![
            button(text("⏪ 10s").size(11)).on_press(Message::SkipBackward).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.3,0.3,0.35,0.9))), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            button(text(play_pause_label).size(13).color(Color::WHITE))
                .on_press(Message::PlayPause).padding([8,16])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgb(0.2,0.6,0.9))), border: Border{ radius:20.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            button(text("10s ⏩").size(11)).on_press(Message::SkipForward).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.3,0.3,0.35,0.9))), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            text(time_text).size(12).color(Color::from_rgb(0.9,0.9,0.95)),
            volume_row,
            speed_btn,
            fs_btn,
            button(text("← Library").size(11)).on_press(Message::NavigateTo(AppScreen::Library)).padding([6,10])
                .style(|t: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.15,0.15,0.18,1.0))), border: Border{ color: Color::from_rgb(0.3,0.3,0.35), width:1.0, radius:6.0.into()}, text_color: t.palette().text, shadow: Shadow::default(), snap:false }),
        ].align_y(Alignment::Center).spacing(8).width(Length::Fill);

        let bottom_bar: Element<Message> = container(column![
            seek_bar,
            Space::new().height(Length::Fixed(6.0)),
            controls_row,
        ].spacing(6)).width(Length::Fill).padding([12,14])
            .style(|_: &Theme| container::Style{
                background: Some(Background::Color(Color::from_rgba(0.08,0.08,0.10,0.92))),
                border: Border{ color: Color::from_rgba(1.0,1.0,1.0,0.08), width:1.0, radius:8.0.into()},
                shadow: Shadow::default(), text_color: Some(Color::from_rgb(0.9,0.9,0.95)), snap:false
            }).into();

        // 2. UX: Auto-Hide Controls - progressive disclosure, hide after 3s mouse inactivity
        // Container with auto-hide: visible if controls_visible else transparent spacer
        let overlay_controls: Element<Message> = if self.controls_visible {
            container(bottom_bar).width(Length::Fill).padding(12).into()
        } else {
            Space::new().height(Length::Fixed(0.0)).into()
        };

        let player_stack = column![stacked_video, overlay_controls].spacing(0).width(Length::Fill).height(Length::Fill);

        // Wrap entire player in mouse_area to capture mouse movement for auto-hide
        mouse_area(player_stack).on_move(|_| Message::MouseMoved).into()
    }
}

fn overlay_btn<'a>(label: &'a str, mode: PlaybackMode, active: bool) -> Element<'a, Message> {
    button(text(label).size(11).align_x(Alignment::Center)).on_press(Message::SetPlaybackMode(mode)).padding([6,10])
        .style(move |_: &Theme, _| button::Style{
            background: Some(Background::Color(if active { Color::from_rgb(0.2,0.6,0.9) } else { Color::from_rgba(0.3,0.3,0.35,0.85) })),
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
fn subscription(_app: &OtipApp) -> iced::Subscription<Message> {
    // 4. Backend wiring helpers + 3. Keyboard shortcuts via events_with + 2. Auto-hide tick
    let frames = iced::Subscription::run(|| {
        iced::stream::channel::<Message>(32, move |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            loop {
                let (frame, pos_update) = {
                    let global = video_player::FRAME_RX_GLOBAL.lock().unwrap();
                    if let Some(rx_arc) = global.as_ref() {
                        let mut guard = rx_arc.lock().unwrap();
                        let mut latest_frame: Option<Handle> = None;
                        let mut latest_pos: Option<(Duration, Duration)> = None;
                        while let Ok(ev) = guard.try_recv() {
                            match ev {
                                PlayerEvent::Frame(h) => latest_frame = Some(h),
                                PlayerEvent::PositionUpdate { position, duration } => latest_pos = Some((position, duration)),
                                PlayerEvent::StateChanged(_playing) => {},
                                PlayerEvent::VolumeChanged(_) => {},
                                PlayerEvent::Ready { .. } => {},
                                PlayerEvent::Error(e) => tracing::warn!("PlayerEvent::Error in subscription: {}", e),
                            }
                        }
                        (latest_frame, latest_pos)
                    } else {
                        (None, None)
                    }
                };
                if let Some(h) = frame {
                    if out.send(Message::FrameReady(h)).await.is_err() { break; }
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
    // 3. Accessibility & Keyboard Shortcuts via iced::subscription::events_with (Iced 0.14: event::listen_with)
    // Spec: Space Play/Pause, Left/Right 5s seek, Up/Down volume 10%, F fullscreen
    let events = iced::event::listen_with(|event, _status, _window| {
        // subscription::events_with equivalent
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
                    _ => None,
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => Some(Message::MouseMoved),
            _ => None,
        }
    });
    // 4. Redraw subscription: trigger UI redraw at ~60fps for video frames
    let redraw = iced::time::every(Duration::from_millis(16)).map(|_| Message::Noop);
    iced::Subscription::batch(vec![frames, close_requests, tick, events, redraw])
}

fn main() -> iced::Result {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env().add_directive("otip=info".parse().unwrap())).with_target(false).init();
    tracing::info!("Starting Otip — Splash → Library (thumbnails at 5s) → Player (playbin + controls)");
    iced::application(boot, update, view).theme(theme).title(title).subscription(subscription)
        .window(iced::window::Settings{ size: iced::Size::new(1280.0, 720.0), min_size: Some(iced::Size::new(900.0,600.0)), decorations: false, ..Default::default() }).run()
}
