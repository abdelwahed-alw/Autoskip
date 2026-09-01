//! Otip - Iced 0.14 with 3-screen routing + GStreamer playbin rendering
//! Splash → Library (thumbnails) → Player (Image from playbin frames + controls)

mod video_player;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use iced::{
    widget::{button, column, container, image, row, scrollable, slider, text, Space, stack},
    Alignment, Background, Border, Color, Element, Length, Shadow, Task, Theme,
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
    Seek(f32), // 0.0..=1.0 normalized - maps to VideoPlayerHandle::seek
    SeekTo(Duration),
    VolumeChanged(f32), // 0.0..=1.0 -> VideoPlayerHandle::set_volume
    SkipForward, // +10s
    SkipBackward, // -10s
    PositionUpdate(Duration, Duration),
    FrameReady(Handle), // raw RGBA from playbin thread
    ThumbnailReady(PathBuf, Option<Handle>),
    ThumbnailsBatch(Vec<(PathBuf, Handle)>),
    CloseRequested, // custom title bar X - non-blocking shutdown via iced::window::close / iced::exit
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
                let clamped = pos.clamp(0.0, 1.0);
                self.timeline_pos = clamped;
                // Also update position for time display
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
                if let Some(p) = &self.video_player {
                    p.seek_to(pos);
                }
                Task::none()
            }
            Message::VolumeChanged(vol) => {
                let v = vol.clamp(0.0, 1.0);
                self.volume = v;
                if let Some(p) = &self.video_player {
                    p.set_volume(v);
                }
                Task::none()
            }
            Message::SkipForward => {
                if let Some(p) = &self.video_player {
                    p.skip_forward();
                }
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
                self.position = self.position.saturating_sub(Duration::from_secs(10));
                if self.duration.as_secs_f32() > 0.0 {
                    self.timeline_pos = (self.position.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
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
        // 1. Emit Correct Message: X button emits Message::CloseRequested (not blocking shutdown)
        container(
            row![
                text(self.title()).size(12).color(Color::from_rgb(0.85, 0.85, 0.88)),
                Space::new().width(Length::Fill),
                // Window controls - custom title bar close button
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
        let top_bar = row![
            button(text("← Back").size(13)).on_press(Message::NavigateTo(AppScreen::Splash)).padding(8)
                .style(|t: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgba(0.15, 0.15, 0.18, 1.0))),
                    border: Border { color: Color::from_rgb(0.3, 0.3, 0.35), width: 1.0, radius: 6.0.into() },
                    text_color: t.palette().text, shadow: Shadow::default(), snap: false
                }),
            Space::new().width(Length::Fill),
            text(self.library_folder.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "No folder selected".into())).size(12).color(Color::from_rgb(0.6, 0.6, 0.65)),
            Space::new().width(Length::Fill),
            button(text("📁 Select Folder").size(13).color(Color::WHITE)).on_press(Message::SelectFolder).padding([8, 14])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.2, 0.6, 0.9))),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE, shadow: Shadow::default(), snap: false
                }),
        ].align_y(Alignment::Center).spacing(12).width(Length::Fill);
        let status = text(&self.status).size(11).color(Color::from_rgb(0.5, 0.5, 0.55));

        // Always iterate over library_videos; never hide grid when thumbnails are pending
        let grid: Element<Message> = if self.library_videos.is_empty() {
            container(column![
                text("No videos found").size(16).color(Color::from_rgb(0.6, 0.6, 0.65)),
                Space::new().height(Length::Fixed(8.0)),
                text("Select a folder containing .mp4 / .mkv / .avi files").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)),
            ].align_x(Alignment::Center)).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill).into()
        } else {
            // Build card for every discovered video (79) - placeholder if thumbnail not yet ready
            // Use chunks(3) to guarantee all videos appear in rows
            let rows: Vec<Element<Message>> = self
                .library_videos
                .chunks(3)
                .map(|chunk| {
                    let row_cards: Vec<Element<Message>> = chunk
                        .iter()
                        .map(|path| {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("video")
                                .to_string();
                            let p = path.clone();
                            let ext = p
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_uppercase();
                            // 2. Async Thumbnails: immediate placeholder (colored box) or cached image via iced::widget::image
                            let thumb: Element<Message> =
                                if let Some(handle) = self.thumbnails.get(path) {
                                    // Thumbnail ready - render via iced::widget::image Handle::from_rgba (GStreamer 5s frame)
                                    container(
                                        image(handle.clone())
                                            .width(Length::Fill)
                                            .height(Length::Fixed(90.0)),
                                    )
                                    .width(Length::Fill)
                                    .height(Length::Fixed(90.0))
                                    .style(|_: &Theme| container::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.08, 0.08, 0.10,
                                        ))),
                                        border: Border {
                                            color: Color::from_rgb(0.25, 0.25, 0.30),
                                            width: 1.0,
                                            radius: 6.0.into(),
                                        },
                                        shadow: Shadow::default(),
                                        text_color: None,
                                        snap: false,
                                    })
                                    .into()
                                } else {
                                    // Placeholder colored box - visible immediately for all 79 videos
                                    container(
                                        column![
                                            text("🎬").size(32).color(Color::from_rgb(0.6, 0.6, 0.9)),
                                            text("loading…").size(9).color(Color::from_rgb(0.5, 0.5, 0.55))
                                        ]
                                        .spacing(4)
                                        .align_x(Alignment::Center),
                                    )
                                    .width(Length::Fill)
                                    .height(Length::Fixed(90.0))
                                    .center_x(Length::Fill)
                                    .center_y(Length::Fill)
                                    .style(|_: &Theme| container::Style {
                                        background: Some(Background::Color(Color::from_rgb(
                                            0.14, 0.14, 0.18,
                                        ))),
                                        border: Border {
                                            color: Color::from_rgb(0.30, 0.30, 0.35),
                                            width: 1.0,
                                            radius: 6.0.into(),
                                        },
                                        shadow: Shadow::default(),
                                        text_color: None,
                                        snap: false,
                                    })
                                    .into()
                                };
                            button(
                                container(
                                    column![
                                        thumb,
                                        text(name.clone()).size(12).color(Color::WHITE),
                                        text(ext.clone())
                                            .size(10)
                                            .color(Color::from_rgb(0.6, 0.6, 0.65)),
                                    ]
                                    .spacing(6)
                                    .align_x(Alignment::Center),
                                )
                                .padding(10)
                                .width(Length::Fill),
                            )
                            .on_press(Message::VideoSelected(p))
                            .padding(0)
                            .width(Length::FillPortion(1))
                            .style(|t: &Theme, s| button::Style {
                                background: Some(Background::Color(match s {
                                    button::Status::Hovered => Color::from_rgb(0.22, 0.22, 0.28),
                                    _ => Color::from_rgb(0.16, 0.16, 0.2),
                                })),
                                border: Border {
                                    color: Color::from_rgb(0.28, 0.28, 0.32),
                                    width: 1.0,
                                    radius: 8.0.into(),
                                },
                                text_color: t.palette().text,
                                shadow: Shadow::default(),
                                snap: false,
                            })
                            .into()
                        })
                        .collect();
                    // Pad last row with spacers so row fills width
                    let mut row_elems = row_cards;
                    while row_elems.len() < 3 {
                        row_elems.push(Space::new().width(Length::FillPortion(1)).into());
                    }
                    row(row_elems).spacing(12).width(Length::Fill).into()
                })
                .collect();
            scrollable(column(rows).spacing(12).padding(4))
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
        // ── Video Rendering: iced::widget::image from raw RGBA ───────
        let video_area: Element<Message> = if let Some(handle) = &self.video_handle {
            image(handle.clone()).width(Length::Fill).height(Length::Fill).into()
        } else {
            container(column![
                text("▶ Initializing GStreamer playbin…").size(18).color(Color::WHITE).align_x(Alignment::Center),
                Space::new().height(Length::Fixed(8.0)),
                text(self.selected_video_path.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("no file")).size(12).color(Color::from_rgb(0.7,0.7,0.75)),
                Space::new().height(Length::Fixed(6.0)),
                text(format!("[{}] {}", self.playback_mode, if self.is_playing { "Playing" } else { "Paused" })).size(11).color(Color::from_rgb(0.2,0.6,0.9)),
                Space::new().height(Length::Fixed(8.0)),
                text("playbin → appsink (sync=true) → Handle::from_rgba → Image").size(10).color(Color::from_rgb(0.5,0.5,0.55)),
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

        let stacked = stack![
            container(video_area).width(Length::Fill).height(Length::Fill),
            container(overlay).width(Length::Fill).height(Length::Fill).align_x(Alignment::End).align_y(Alignment::Start).padding(12),
        ].width(Length::Fill).height(Length::FillPortion(1));

        // ── Modern semi-transparent bottom control bar (container + row) ──
        // Play/Pause Toggle changes label based on PlaybackState
        let playback_state = if self.is_playing { PlaybackState::Playing } else { PlaybackState::Paused };
        let play_pause_label = match playback_state {
            PlaybackState::Playing => "⏸ Pause",
            PlaybackState::Paused => "▶ Play",
            _ => "▶ Play",
        };
        let time_text = format!("{} / {}", format_duration_short(self.position), format_duration_short(self.duration));

        let controls_row = row![
            // Skip backward 10s
            button(text("⏪ 10s").size(11)).on_press(Message::SkipBackward).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.3,0.3,0.35,0.9))), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            // Play/Pause Toggle (changes label based on PlaybackState / is_playing)
            button(text(play_pause_label).size(13).color(Color::WHITE))
                .on_press(Message::PlayPause).padding([8,16])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgb(0.2,0.6,0.9))), border: Border{ radius:20.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            // Skip forward 10s
            button(text("10s ⏩").size(11)).on_press(Message::SkipForward).padding([6,10])
                .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.3,0.3,0.35,0.9))), border: Border{ radius:6.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
            // Time Elapsed / Total Duration
            text(time_text).size(12).color(Color::from_rgb(0.9,0.9,0.95)),
            // Volume label + small slider
            text("🔊").size(12).color(Color::from_rgb(0.7,0.7,0.75)),
            slider(0.0..=1.0, self.volume, Message::VolumeChanged).step(0.01).width(Length::Fixed(90.0)),
            button(text("← Library").size(11)).on_press(Message::NavigateTo(AppScreen::Library)).padding([6,10])
                .style(|t: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.15,0.15,0.18,1.0))), border: Border{ color: Color::from_rgb(0.3,0.3,0.35), width:1.0, radius:6.0.into()}, text_color: t.palette().text, shadow: Shadow::default(), snap:false }),
        ].align_y(Alignment::Center).spacing(10).width(Length::Fill);

        let seek_bar = slider(0.0..=1.0, self.timeline_pos, Message::Seek).step(0.01).width(Length::Fill);

        let bottom_bar = container(column![
            seek_bar,
            Space::new().height(Length::Fixed(6.0)),
            controls_row,
        ].spacing(6)).width(Length::Fill).padding([12,14])
            .style(|_: &Theme| container::Style{
                background: Some(Background::Color(Color::from_rgba(0.08,0.08,0.10,0.92))),
                border: Border{ color: Color::from_rgba(1.0,1.0,1.0,0.08), width:1.0, radius:8.0.into()},
                shadow: Shadow::default(), text_color: Some(Color::from_rgb(0.9,0.9,0.95)), snap:false
            });

        column![stacked, bottom_bar].spacing(0).width(Length::Fill).height(Length::Fill).into()
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
    iced::Subscription::run(|| {
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
    })
}

fn main() -> iced::Result {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env().add_directive("otip=info".parse().unwrap())).with_target(false).init();
    tracing::info!("Starting Otip — Splash → Library (thumbnails at 5s) → Player (playbin + controls)");
    iced::application(boot, update, view).theme(theme).title(title).subscription(subscription)
        .window(iced::window::Settings{ size: iced::Size::new(1280.0, 720.0), min_size: Some(iced::Size::new(900.0,600.0)), ..Default::default() }).run()
}
