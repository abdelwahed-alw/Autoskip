//! Main player screen - Iced 0.14 with modern controls
//! Semi-transparent bottom control bar: play/pause, time, seek slider, volume, skip
use std::time::Duration;
use iced::{
    widget::{button, column, container, row, slider, text, Space},
    Alignment, Element, Length, Theme, Color, Border, Background, Shadow,
};
use otip_core::domain::PlaybackState;
use otip_core::timeline::format_duration_short;
use crate::state::{AppState, Message};
use crate::widgets::timeline::{TimelineWidget, TimelineMessage};
use crate::widgets::video_display::{VideoDisplay, VideoDisplayMessage};

pub struct PlayerScreen {
    timeline_widget: TimelineWidget,
    video_display: VideoDisplay,
}
impl PlayerScreen {
    pub fn new() -> Self {
        Self {
            timeline_widget: TimelineWidget::new(),
            video_display: VideoDisplay::new(),
        }
    }
    pub fn view<'a>(&'a self, state: &'a AppState) -> Element<'a, Message> {
        // Seek progress 0.0..1.0 bound to duration/position
        let progress = if state.duration.as_secs_f32() > 0.0 {
            (state.position.as_secs_f32() / state.duration.as_secs_f32()).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Play/Pause label depends on PlaybackState per spec
        let play_label = match state.playback_state {
            PlaybackState::Playing => "⏸ Pause",
            PlaybackState::Paused => "▶ Play",
            PlaybackState::Buffering => "⏳ Buffering",
            PlaybackState::Ended => "↺ Replay",
            PlaybackState::Error => "⚠ Error",
            _ => "▶ Play",
        };

        // Timeline widget (visual color indicators)
        let timeline = self
            .timeline_widget
            .view(
                state.timeline.as_deref(),
                state.position,
                state.duration,
                state.hover_position,
                state.is_seeking,
            )
            .map(|m| match m {
                TimelineMessage::Seek(p) => Message::SeekTo(p),
                TimelineMessage::Hover(p) => Message::TimelineHover(p),
                TimelineMessage::DragStart => Message::TimelineSeekStart,
                TimelineMessage::DragEnd(p) => Message::TimelineSeekEnd(p),
            });

        // Video area
        let video_area = container(
            self.video_display
                .view()
                .map(|m| match m {
                    VideoDisplayMessage::DoubleClick => Message::ToggleFullscreen,
                    VideoDisplayMessage::RightClick => Message::ShowSettings(true),
                }),
        )
        .width(Length::Fill)
        .height(Length::FillPortion(1))
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgb(0.05, 0.05, 0.08))),
            border: Border::default(),
            shadow: Shadow::default(),
            text_color: Some(Color::from_rgb(0.9, 0.9, 0.95)),
            snap: false,
        });

        // ── Modern semi-transparent bottom control bar (container + row) ──
        // Seek Bar: slider spanning width, bound to total duration and current position
        let seek_bar = slider(0.0..=1.0, progress, Message::Seek)
            .step(0.005)
            .width(Length::Fill);

        // Time Elapsed / Total Duration: text widget e.g. "01:23 / 15:00"
        let time_label = text(format!(
            "{} / {}",
            format_duration_short(state.position),
            format_duration_short(state.duration)
        ))
        .size(12)
        .color(Color::from_rgb(0.9, 0.9, 0.95));

        // Controls row: Play/Pause Toggle, Skip Buttons, Time, Volume
        let controls_row = row![
            // Skip backward 10s
            button(text("⏪ -10s").size(11))
                .on_press(Message::SeekTo(state.position.saturating_sub(Duration::from_secs(10))))
                .padding([6, 10])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgba(0.3, 0.3, 0.35, 0.9))),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false
                }),
            // Play/Pause Toggle - label changes with PlaybackState
            button(text(play_label).size(12).color(Color::WHITE))
                .on_press(Message::PlayPause)
                .padding([8, 16])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.2, 0.6, 0.9))),
                    border: Border { radius: 20.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false
                }),
            // Skip forward 10s
            button(text("+10s ⏩").size(11))
                .on_press(Message::SeekTo(
                    (state.position + Duration::from_secs(10)).min(state.duration)
                ))
                .padding([6, 10])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgba(0.3, 0.3, 0.35, 0.9))),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false
                }),
            time_label,
            Space::new().width(Length::Fill),
            // Volume Control: smaller slider 0.0..1.0
            text("🔊").size(12).color(Color::from_rgb(0.7, 0.7, 0.75)),
            slider(0.0..=1.0, state.volume, Message::VolumeChanged)
                .step(0.02)
                .width(Length::Fixed(90.0)),
            button(text("⛶").size(13))
                .on_press(Message::ToggleFullscreen)
                .padding([6, 10])
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgba(0.3, 0.3, 0.35, 0.9))),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false
                }),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill);

        let bottom_bar = container(column![seek_bar, Space::new().height(Length::Fixed(4.0)), controls_row].spacing(6))
            .width(Length::Fill)
            .padding([12, 14])
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.08, 0.08, 0.10, 0.92))),
                border: Border {
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: Shadow::default(),
                text_color: Some(Color::from_rgb(0.9, 0.9, 0.95)),
                snap: false,
            });

        column![video_area, timeline, bottom_bar]
            .spacing(8)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
impl Default for PlayerScreen {
    fn default() -> Self {
        Self::new()
    }
}
