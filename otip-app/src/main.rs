//! Otip - Iced 0.14 with 3-screen routing + MPV video rendering
//! Splash → Library (auto-scan) → Player (Image from mpv frames)

mod video_player;

use std::path::PathBuf;
use iced::{
    widget::{button, column, container, image, row, scrollable, slider, text, Space, stack},
    Alignment, Background, Border, Color, Element, Length, Shadow, Task, Theme,
};
use iced::widget::image::Handle;
use otip_core::domain::PlaybackMode;
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
    Seek(f32),
    FrameReady(Handle), // ← raw RGBA from mpv thread
    MpvError(String),
    Noop,
}

pub struct OtipApp {
    screen: AppScreen,
    library_folder: Option<PathBuf>,
    library_videos: Vec<PathBuf>,
    selected_video_path: Option<PathBuf>,
    playback_mode: PlaybackMode,
    is_playing: bool,
    timeline_pos: f32,
    status: String,
    // ── MPV integration ──
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
                selected_video_path: None,
                playback_mode: PlaybackMode::SafeMode,
                is_playing: false,
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
                self.library_videos = videos;
                self.status = if self.library_videos.is_empty() {
                    "No videos found in Videos/Downloads — use Select Folder".into()
                } else {
                    format!("Auto-discovered {} videos", self.library_videos.len())
                };
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
                            self.library_videos = videos;
                        }
                        Err(e) => {
                            self.status = format!("Failed to read folder: {}", e);
                            self.library_videos.clear();
                        }
                    }
                }
                Task::none()
            }
            // ── Engine Initialization (background, non-blocking) ───────
            Message::VideoSelected(path) => {
                self.selected_video_path = Some(path.clone());
                self.playback_mode = self.playback_mode; // keep current
                self.screen = AppScreen::Player;
                self.is_playing = true;
                self.video_handle = None;
                // Spawn MPV in background thread; UI stays responsive
                let player = VideoPlayerHandle::spawn(path.clone());
                self.video_player = Some(player);
                self.status = format!("Loading: {}", path.display());
                tracing::info!("VideoSelected {:?} with mode {} → Player", path, self.playback_mode);
                Task::none()
            }
            Message::FileSelected(opt) => {
                if let Some(p) = opt {
                    // Reuse VideoSelected path so mpv init is centralized
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
                self.timeline_pos = pos.clamp(0.0, 1.0);
                if let Some(p) = &self.video_player {
                    p.seek(pos);
                }
                Task::none()
            }
            Message::FrameReady(handle) => {
                self.video_handle = Some(handle);
                // Update timeline pos from frame progress if needed
                Task::none()
            }
            Message::MpvError(e) => {
                self.status = format!("MPV error: {}", e);
                Task::none()
            }
            Message::Noop => Task::none(),
        }
    }

    fn view(&self) -> Element<Message> {
        let content = match self.screen {
            AppScreen::Splash => self.view_splash(),
            AppScreen::Library => self.view_library(),
            AppScreen::Player => self.view_player(),
        };
        container(content)
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
        let grid: Element<Message> = if self.library_videos.is_empty() {
            container(column![
                text("No videos found").size(16).color(Color::from_rgb(0.6, 0.6, 0.65)),
                Space::new().height(Length::Fixed(8.0)),
                text("Select a folder containing .mp4 / .mkv / .avi files").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)),
            ].align_x(Alignment::Center)).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill).into()
        } else {
            let cards = self.library_videos.iter().map(|path| {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("video").to_string();
                let p = path.clone();
                button(container(column![
                    container(text("🎬").size(32)).width(Length::Fill).center_x(Length::Fill),
                    text(name.clone()).size(12).color(Color::WHITE),
                    text(p.extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase()).size(10).color(Color::from_rgb(0.6, 0.6, 0.65)),
                ].spacing(6).align_x(Alignment::Center)).padding(12).width(Length::Fill))
                .on_press(Message::VideoSelected(p)).padding(0).width(Length::FillPortion(1))
                .style(|t: &Theme, s| button::Style {
                    background: Some(Background::Color(match s {
                        button::Status::Hovered => Color::from_rgb(0.22, 0.22, 0.28),
                        _ => Color::from_rgb(0.16, 0.16, 0.2),
                    })),
                    border: Border { color: Color::from_rgb(0.28, 0.28, 0.32), width: 1.0, radius: 8.0.into() },
                    text_color: t.palette().text, shadow: Shadow::default(), snap: false
                }).into()
            });
            let mut rows: Vec<Element<Message>> = Vec::new();
            let mut current_row: Vec<Element<Message>> = Vec::new();
            for (i, card) in cards.enumerate() {
                current_row.push(card);
                if (i + 1) % 3 == 0 {
                    rows.push(row(std::mem::take(&mut current_row)).spacing(12).width(Length::Fill).into());
                }
            }
            if !current_row.is_empty() {
                while current_row.len() < 3 { current_row.push(Space::new().width(Length::FillPortion(1)).into()); }
                rows.push(row(current_row).spacing(12).width(Length::Fill).into());
            }
            scrollable(column(rows).spacing(12).padding(4)).width(Length::Fill).height(Length::Fill).into()
        };
        container(column![top_bar, Space::new().height(Length::Fixed(12.0)), status, Space::new().height(Length::Fixed(8.0)), grid].spacing(4))
            .width(Length::Fill).height(Length::Fill).padding(16)
            .style(|t: &Theme| container::Style {
                background: Some(Background::Color(t.palette().background)),
                border: Border::default(), shadow: Shadow::default(), text_color: Some(t.palette().text), snap: false,
            }).into()
    }

    fn view_player(&self) -> Element<Message> {
        // ── Video Rendering: iced::widget::image from raw RGBA ───────
        // Frame extraction runs in video_player.rs:spawn_blocking → mpsc → Subscription → FrameReady(Handle)
        let video_area: Element<Message> = if let Some(handle) = &self.video_handle {
            // Real video frame — Handle::from_rgba(w,h,rgba) created in background thread
            image(handle.clone()).width(Length::Fill).height(Length::Fill).into()
        } else {
            container(column![
                text("▶ Initializing MPV…").size(18).color(Color::WHITE).align_x(Alignment::Center),
                Space::new().height(Length::Fixed(8.0)),
                text(self.selected_video_path.as_ref().and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or("no file")).size(12).color(Color::from_rgb(0.7,0.7,0.75)),
                Space::new().height(Length::Fixed(6.0)),
                text(format!("[{}] {}", self.playback_mode, if self.is_playing { "Playing" } else { "Paused" })).size(11).color(Color::from_rgb(0.2,0.6,0.9)),
                Space::new().height(Length::Fixed(8.0)),
                text("mpv → spawn_blocking → Handle::from_rgba → Image").size(10).color(Color::from_rgb(0.5,0.5,0.55)),
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

        let bottom_bar = container(column![
            slider(0.0..=1.0, self.timeline_pos, Message::Seek).step(0.01).width(Length::Fill),
            row![
                button(text("← Back to Library").size(12)).on_press(Message::NavigateTo(AppScreen::Library)).padding([6,10])
                    .style(|t: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgba(0.15,0.15,0.18,1.0))), border: Border{ color: Color::from_rgb(0.3,0.3,0.35), width:1.0, radius:6.0.into()}, text_color: t.palette().text, shadow: Shadow::default(), snap:false }),
                Space::new().width(Length::Fill),
                button(text(if self.is_playing { "⏸ Pause" } else { "▶ Play" }).size(13).color(Color::WHITE))
                    .on_press(Message::PlayPause).padding([8,16])
                    .style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgb(0.2,0.6,0.9))), border: Border{ radius:20.0.into(), ..Default::default()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }),
                Space::new().width(Length::Fill),
                text(format!("{:.0}%", self.timeline_pos * 100.0)).size(11).color(Color::from_rgb(0.6,0.6,0.65)),
            ].align_y(Alignment::Center).spacing(12).width(Length::Fill),
        ].spacing(8)).width(Length::Fill).padding([10,12])
            .style(|t: &Theme| container::Style{ background: Some(Background::Color(Color::from_rgb(0.12,0.12,0.15))), border: Border{ color: Color::from_rgb(0.22,0.22,0.26), width:1.0, radius:8.0.into()}, shadow: Shadow::default(), text_color: Some(t.palette().text), snap:false });

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
        const EXTS: &[&str] = &["mp4","mkv","avi"];
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
        iced::stream::channel::<Message>(4, move |mut out: iced::futures::channel::mpsc::Sender<Message>| async move {
            loop {
                let frame = video_player::FRAME_RX_GLOBAL.get().and_then(|rx| {
                    let mut guard = rx.lock().unwrap();
                    let mut next = None;
                    while let Ok(ev) = guard.try_recv() {
                        if let PlayerEvent::Frame(h) = ev { next = Some(h); break; }
                    }
                    next
                });
                if let Some(h) = frame {
                    let _ = out.send(Message::FrameReady(h)).await;
                }
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            }
        })
    })
}

fn main() -> iced::Result {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env().add_directive("otip=info".parse().unwrap())).with_target(false).init();
    tracing::info!("Starting Otip — Splash → Library → Player (MPV → Image)");
    iced::application(boot, update, view).theme(theme).title(title).subscription(subscription)
        .window(iced::window::Settings{ size: iced::Size::new(1280.0, 720.0), min_size: Some(iced::Size::new(900.0,600.0)), ..Default::default() }).run()
}
