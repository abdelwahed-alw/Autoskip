//! Main application - Iced 0.14

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use iced::{
    widget::{container, row, column, text, Space, button},
    Alignment, Element, Length, Theme, Color, Border, Background, Shadow, Task, Subscription,
    keyboard::key::Named,
};
use otip_core::domain::{UserPreferences, PlaybackMode, PlaybackState};
use otip_core::config::AppConfig;
use otip_core::events::BackendEvent;
use crate::state::{AppState, Message};
use crate::screens::{ModeSelectionScreen, PlayerScreen, SettingsScreen};
use crate::worker::{WorkerHandle, create_video_engine};
use crate::theme::get_theme;
use otip_ai::moderator::{ContentModerator, ModerationConfig};
use otip_ai::gemini::GeminiConfig;
use tracing::{info, error};

/// Main application state
pub struct OtipApp {
    state: AppState,
    worker: WorkerHandle,
    mode_selection: ModeSelectionScreen,
    player_screen: PlayerScreen,
    config: AppConfig,
}

impl OtipApp {
    fn new() -> (Self, Task<Message>) {
        let config = AppConfig::load().unwrap_or_default();
        let preferences = config.preferences.clone();
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = AppState::new(command_tx.clone(), event_rx, preferences);
        // Initialize Gemini state from config (API key + model)
        state.set_gemini_config(config.gemini_api_key.clone(), config.gemini_model.clone());
        let mut worker = WorkerHandle::new(command_tx, event_tx);
        let video_engine = create_video_engine();
        worker.set_video_engine(video_engine);
        // Build moderator with Gemini config from persisted settings (default: 3.7 Flash)
        let (scan_progress_tx, _scan_progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let (segment_tx, _segment_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut moderation_config = ModerationConfig::default();
        moderation_config.gemini_config = GeminiConfig {
            api_key: config.get_gemini_api_key().unwrap_or_default(),
            model: config.gemini_model.clone(),
            endpoint: config.gemini_endpoint.clone(),
            ..Default::default()
        };
        let moderator = ContentModerator::new(moderation_config, scan_progress_tx, segment_tx)
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to create moderator with saved config, using default: {}", e);
                let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
                let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
                ContentModerator::new(ModerationConfig::default(), tx1, tx2).expect("default moderator")
            });
        worker.set_content_moderator(moderator);
        state.video_engine = worker.video_engine.clone();
        state.content_moderator = worker.content_moderator.clone();
        let app = Self {
            state,
            worker,
            mode_selection: ModeSelectionScreen::new(PlaybackMode::SafeMode),
            player_screen: PlayerScreen::new(),
            config,
        };
        tokio::spawn(Self::process_commands(app.worker.command_tx.clone(), command_rx));
        (app, Task::none())
    }

    fn title(&self) -> String {
        if let Some(metadata) = &self.state.video_metadata {
            format!("Otip - {}", metadata.title)
        } else {
            "Otip - Smart Video Player".to_string()
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Open Video")
                        .add_filter("Video", &["mp4", "mkv", "avi", "mov", "webm", "flv", "wmv", "m4v"])
                        .pick_file()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::FileSelected,
            ),
            Message::FileSelected(path_opt) => {
                if let Some(path) = path_opt {
                    self.state.open_video(path);
                    self.mode_selection = ModeSelectionScreen::new(self.state.selected_mode);
                }
                Task::none()
            }
            Message::ModeSelected(mode) => {
                self.state.selected_mode = mode;
                self.mode_selection.set_selected_mode(mode);
                Task::none()
            }
            Message::ConfirmMode => {
                self.state.confirm_mode_selection();
                Task::none()
            }
            Message::CancelMode => {
                self.state.cancel_mode_selection();
                Task::none()
            }
            Message::PlayPause => {
                self.state.toggle_playback();
                Task::none()
            }
            Message::Stop => {
                if let Some(video_id) = self.state.current_video {
                    let _ = self.state.command_tx.send(otip_core::events::UiCommand::Stop(video_id));
                }
                Task::none()
            }
            Message::Seek(progress) => {
                let position = Duration::from_secs_f32(self.state.duration.as_secs_f32() * progress);
                self.state.seek(position);
                Task::none()
            }
            Message::SeekTo(position) => {
                self.state.seek(position);
                Task::none()
            }
            Message::VolumeChanged(volume) => {
                self.state.set_volume(volume);
                Task::none()
            }
            Message::PlaybackRateChanged(rate) => {
                self.state.set_playback_rate(rate);
                Task::none()
            }
            Message::ToggleFullscreen => {
                self.state.toggle_fullscreen();
                Task::none()
            }
            Message::ShowSettings(show) => {
                self.state.show_settings = show;
                Task::none()
            }
            Message::ShowPlaylist(show) => {
                self.state.show_playlist = show;
                Task::none()
            }
            Message::TogglePlaylist => {
                self.state.show_playlist = !self.state.show_playlist;
                Task::none()
            }
            Message::WindowResized(width, height) => {
                self.state.window_width = width;
                self.state.window_height = height;
                Task::none()
            }
            Message::TimelineHover(pos) => {
                self.state.hover_position = pos;
                Task::none()
            }
            Message::TimelineSeekStart => {
                self.state.is_seeking = true;
                Task::none()
            }
            Message::TimelineSeekEnd(pos) => {
                self.state.is_seeking = false;
                self.state.seek(pos);
                Task::none()
            }
            Message::PreferencesChanged(prefs) => {
                self.state.preferences = prefs.clone();
                self.config.preferences = prefs;
                let _ = self.config.save();
                Task::none()
            }
            Message::ApiKeyChanged(key) => {
                self.state.gemini_api_key = key;
                Task::none()
            }
            Message::ApiKeyVisibilityToggled => {
                self.state.gemini_api_key_visible = !self.state.gemini_api_key_visible;
                Task::none()
            }
            Message::GeminiModelChanged(model) => {
                self.state.gemini_model = model;
                Task::none()
            }
            Message::SaveGeminiConfig => {
                // Persist to config file
                let api_key_opt = if self.state.gemini_api_key.trim().is_empty() {
                    None
                } else {
                    Some(self.state.gemini_api_key.trim().to_string())
                };
                self.config.gemini_api_key = api_key_opt.clone();
                self.config.gemini_model = self.state.gemini_model.clone();
                // Also mirror to preferences.api_key for backward compat
                self.config.preferences.api_key = api_key_opt.clone();
                self.state.preferences.api_key = api_key_opt.clone();

                let save_result = self.config.save();
                if let Err(e) = &save_result {
                    self.state.scan_error = Some(format!("Failed to save config: {}", e));
                    return Task::none();
                }

                // Recreate content moderator with new Gemini config so next scan uses new key/model
                let (scan_progress_tx, _scan_progress_rx) = tokio::sync::mpsc::unbounded_channel();
                let (segment_tx, _segment_rx) = tokio::sync::mpsc::unbounded_channel();
                let mut moderation_config = ModerationConfig::default();
                moderation_config.gemini_config = GeminiConfig {
                    api_key: self.state.gemini_api_key.clone(),
                    model: self.state.gemini_model.clone(),
                    endpoint: self.config.gemini_endpoint.clone(),
                    ..Default::default()
                };
                match ContentModerator::new(moderation_config, scan_progress_tx, segment_tx) {
                    Ok(moderator) => {
                        self.worker.set_content_moderator(moderator);
                        self.state.content_moderator = self.worker.content_moderator.clone();
                        self.state.scan_error = None;
                        info!("Gemini config saved: model={}, has_key={}", self.state.gemini_model, !self.state.gemini_api_key.is_empty());
                    }
                    Err(e) => {
                        self.state.scan_error = Some(format!("Failed to apply Gemini config: {}", e));
                    }
                }
                Task::none()
            }
            Message::GeminiConfigSaved(result) => {
                match result {
                    Ok(()) => self.state.scan_error = None,
                    Err(e) => self.state.scan_error = Some(e),
                }
                Task::none()
            }
            Message::Tick => {
                self.update_position_from_engine();
                Task::none()
            }
            Message::BackendEvent(event) => {
                self.handle_backend_event(event);
                Task::none()
            }
            Message::Shutdown => {
                std::process::exit(0)
            }
            Message::ScanProgress(progress) => {
                self.state.update_scan_progress(progress);
                Task::none()
            }
            Message::ScanComplete(segments) => {
                self.state.is_scanning = false;
                if let Some(timeline) = &self.state.timeline {
                    for segment in segments {
                        timeline.add_segment(segment);
                    }
                }
                if self.state.selected_mode == PlaybackMode::SafeMode {
                    if let Some(video_id) = self.state.current_video {
                        if let Some(engine) = &self.state.video_engine {
                            let engine_clone = engine.clone();
                            tokio::spawn(async move {
                                let _ = engine_clone.lock().await.play(video_id).await;
                            });
                        }
                    }
                }
                Task::none()
            }
            Message::ScanError(error) => {
                self.state.scan_error = Some(error);
                self.state.is_scanning = false;
                Task::none()
            }
            Message::NewScanSegment(segment) => {
                self.state.add_timeline_segment(segment);
                Task::none()
            }
            Message::PositionUpdate(position, duration) => {
                self.state.update_position(position, duration);
                Task::none()
            }
            Message::StateChanged(state) => {
                self.state.update_playback_state(state);
                Task::none()
            }
            Message::VideoOpened(video_id, metadata) => {
                self.state.current_video = Some(video_id);
                self.state.video_metadata = Some(metadata.clone());
                self.state.duration = metadata.duration;
                let timeline = Arc::new(otip_core::timeline::Timeline::new(video_id, metadata.duration));
                self.state.set_timeline(timeline);
                self.state.playback_state = PlaybackState::Playing;
                Task::none()
            }
            Message::VideoOpenFailed(error) => {
                self.state.scan_error = Some(format!("Failed to open video: {}", error));
                Task::none()
            }
            Message::AutoSkipTriggered(from, to) => {
                info!("Auto-skip triggered: {:?} -> {:?}", from, to);
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<Message> {
        let content = if self.state.show_settings {
            SettingsScreen::view(&self.state)
        } else if self.state.show_mode_selection {
            self.mode_selection.view(
                self.state.pending_video_path.as_ref().and_then(|p| p.file_stem()).and_then(|s| s.to_str()).unwrap_or("Unknown")
            )
        } else if self.state.current_video.is_some() {
            self.player_screen_view()
        } else {
            self.welcome_screen()
        };
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().background)),
                border: Border::default(),
                shadow: Shadow::default(),
                text_color: Some(theme.palette().text),
            })
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick);
        let keyboard = iced::event::listen_with(|event, _status, _window| match event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if modifiers.command() || modifiers.control() {
                    return None;
                }
                match key.as_ref() {
                    iced::keyboard::Key::Named(Named::Space) => Some(Message::PlayPause),
                    iced::keyboard::Key::Named(Named::ArrowLeft) => {
                        if modifiers.shift() {
                            Some(Message::SeekTo(self.state.position.saturating_sub(Duration::from_secs(60))))
                        } else {
                            Some(Message::SeekTo(self.state.position.saturating_sub(Duration::from_secs(10))))
                        }
                    }
                    iced::keyboard::Key::Named(Named::ArrowRight) => {
                        if modifiers.shift() {
                            Some(Message::SeekTo(self.state.position + Duration::from_secs(60)))
                        } else {
                            Some(Message::SeekTo(self.state.position + Duration::from_secs(10)))
                        }
                    }
                    iced::keyboard::Key::Named(Named::ArrowUp) => Some(Message::VolumeChanged((self.state.volume + 0.05).min(1.0))),
                    iced::keyboard::Key::Named(Named::ArrowDown) => Some(Message::VolumeChanged((self.state.volume - 0.05).max(0.0))),
                    iced::keyboard::Key::Named(Named::F) => Some(Message::ToggleFullscreen),
                    iced::keyboard::Key::Named(Named::Escape) => {
                        if self.state.is_fullscreen {
                            Some(Message::ToggleFullscreen)
                        } else if self.state.show_settings {
                            Some(Message::ShowSettings(false))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        });
        Subscription::batch(vec![tick, keyboard])
    }

    fn player_screen_view(&self) -> Element<Message> {
        self.player_screen.view(&self.state)
    }

    fn theme(&self) -> Theme {
        get_theme(self.state.preferences.theme)
    }

    fn welcome_screen(&self) -> Element<Message> {
        let content = column![
            Space::new(Length::Shrink, Length::Fixed(Length::FillPortion(1))),
            column![
                text("🎬").size(80).color(Color::from_rgb(0.2, 0.6, 0.9)),
                Space::new(Length::Shrink, Length::Fixed(16)),
                text("Otip").size(48).color(Color::from_rgb(0.95, 0.95, 0.95)),
                Space::new(Length::Shrink, Length::Fixed(8)),
                text("Smart Video Player with AI Content Moderation").size(18).color(Color::from_rgb(0.7, 0.7, 0.75)),
            ].align_x(Alignment::Center),
            Space::new(Length::Shrink, Length::Fixed(48)),
            row![
                feature_card("🛡️", "Safe Mode", "Full scan before playback"),
                feature_card("⚡", "Instant Play", "Background scanning"),
                feature_card("🎯", "Auto-Skip", "Seamless content filtering"),
                feature_card("🔧", "Hardware Accel", "GPU-accelerated decoding"),
            ].spacing(16).width(Length::Fill),
            Space::new(Length::Shrink, Length::Fixed(48)),
            button(
                container(row![text("📁").size(20), Space::new(Length::Fixed(8), Length::Shrink), text("Open Video").size(18)].align_y(Alignment::Center))
                    .width(Length::Fixed(200.0)).center_x(Length::Fill).padding(16)
            ).on_press(Message::OpenFile).padding(0),
            Space::new(Length::Shrink, Length::Fixed(Length::FillPortion(1))),
        ].align_x(Alignment::Center).width(Length::Fill).height(Length::Fill);
        container(content).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill).into()
    }

    fn handle_backend_event(&mut self, event: BackendEvent) {
        match event {
            BackendEvent::VideoOpened { video_id, metadata } => {
                self.state.current_video = Some(video_id);
                self.state.video_metadata = Some(metadata.clone());
                self.state.duration = metadata.duration;
                let timeline = Arc::new(otip_core::timeline::Timeline::new(video_id, metadata.duration));
                self.state.set_timeline(timeline);
                self.state.playback_state = PlaybackState::Playing;
            }
            BackendEvent::VideoOpenFailed { path: _, error } => {
                self.state.scan_error = Some(format!("Failed to open video: {}", error));
            }
            BackendEvent::PlaybackStateChanged { video_id: _, state } => {
                self.state.update_playback_state(state);
            }
            BackendEvent::PositionUpdate { video_id: _, position, duration } => {
                self.state.update_position(position, duration);
            }
            BackendEvent::VolumeChanged { video_id: _, volume } => {
                self.state.volume = volume;
            }
            BackendEvent::ScanProgress(progress) => {
                self.state.update_scan_progress(progress);
            }
            BackendEvent::ScanComplete { video_id: _, segments } => {
                self.state.is_scanning = false;
                if let Some(timeline) = &self.state.timeline {
                    for segment in segments { timeline.add_segment(segment); }
                }
            }
            BackendEvent::ScanError { video_id: _, error } => {
                self.state.scan_error = Some(error);
                self.state.is_scanning = false;
            }
            BackendEvent::NewScanSegment { video_id: _, segment } => {
                self.state.add_timeline_segment(segment);
            }
            BackendEvent::AutoSkipTriggered { video_id: _, from, to } => {
                info!("Auto-skip triggered: {:?} -> {:?}", from, to);
            }
            BackendEvent::FrameReady { .. } => {}
            BackendEvent::Error { message, .. } => {
                error!("Backend error: {}", message);
                self.state.scan_error = Some(message);
            }
        }
    }

    fn update_position_from_engine(&mut self) {
        if let (Some(video_id), Some(engine)) = (self.state.current_video, &self.state.video_engine) {
            let engine_clone = engine.clone();
            let event_tx = self.worker.event_tx.clone();
            tokio::spawn(async move {
                if let Ok((position, duration)) = engine_clone.lock().await.get_position(video_id).await {
                    let _ = event_tx.send(BackendEvent::PositionUpdate { video_id, position, duration });
                }
                if let Ok(state) = engine_clone.lock().await.get_state(video_id).await {
                    let _ = event_tx.send(BackendEvent::PlaybackStateChanged { video_id, state });
                }
            });
        }
    }

    async fn process_commands(command_tx: tokio::sync::mpsc::UnboundedSender<otip_core::events::UiCommand>, mut command_rx: tokio::sync::mpsc::UnboundedReceiver<otip_core::events::UiCommand>) {
        while let Some(cmd) = command_rx.recv().await {
            let _ = command_tx.send(cmd);
        }
    }
}

fn feature_card<'a>(icon: &'a str, title: &'a str, desc: &'a str) -> Element<'a, Message> {
    container(column![text(icon).size(32), Space::new(Length::Shrink, Length::Fixed(8)), text(title).size(16).color(Color::from_rgb(0.95, 0.95, 0.95)), Space::new(Length::Shrink, Length::Fixed(4)), text(desc).size(13).color(Color::from_rgb(0.6, 0.6, 0.65))].align_x(Alignment::Center))
        .width(Length::FillPortion(1)).padding(20)
        .style(|theme: &Theme| container::Style { background: Some(Background::Color(theme.palette().background)), border: Border::default(), shadow: Shadow::default(), text_color: Some(theme.palette().text) }).into()
}

/// Boot function for iced::application
fn boot() -> (OtipApp, Task<Message>) {
    OtipApp::new()
}
fn update(app: &mut OtipApp, msg: Message) -> Task<Message> { app.update(msg) }
fn view(app: &OtipApp) -> Element<Message> { app.view() }
fn theme(app: &OtipApp) -> Theme { app.theme() }
fn title(app: &OtipApp) -> String { app.title() }
fn subscription(app: &OtipApp) -> Subscription<Message> { app.subscription() }

/// Run the application - Iced 0.14
pub fn run() -> iced::Result {
    iced::application(boot, update, view)
        .subscription(subscription)
        .theme(theme)
        .title(title)
        .window(iced::window::Settings { size: iced::Size::new(1280.0, 720.0), min_size: Some(iced::Size::new(800.0, 600.0)), ..Default::default() })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_app_creation() {}
}
