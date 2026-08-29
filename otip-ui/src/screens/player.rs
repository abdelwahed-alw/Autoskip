//! Main player screen - minimal 0.14
use std::time::Duration;
use iced::{widget::{button, column, container, row, text, Space}, Alignment, Element, Length, Theme, Color, Border, Background, Shadow};
use otip_core::domain::PlaybackState;
use otip_core::timeline::format_duration_short;
use crate::state::{AppState, Message};
use crate::widgets::timeline::{TimelineWidget, TimelineMessage};
use crate::widgets::video_display::{VideoDisplay, VideoDisplayMessage};
pub struct PlayerScreen { timeline_widget: TimelineWidget, video_display: VideoDisplay }
impl PlayerScreen {
    pub fn new() -> Self { Self { timeline_widget: TimelineWidget::new(), video_display: VideoDisplay::new() } }
    pub fn view(&mut self, state: &AppState) -> Element<Message> {
        let controls = row![
            button(text("Play/Pause")).style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgb(0.2,0.6,0.9))), border: Border::default(), text_color: Color::WHITE, shadow: Shadow::default(), snap:false }).on_press(Message::PlayPause),
            text(format_duration_short(state.position)).size(13),
            text(format_duration_short(state.duration)).size(13),
            button(text("Fullscreen")).on_press(Message::ToggleFullscreen),
        ].spacing(12).align_y(Alignment::Center);
        let timeline = self.timeline_widget.view(state.timeline.as_deref(), state.position, state.duration, state.hover_position, state.is_seeking)
            .map(|m| match m { TimelineMessage::Seek(p)=>Message::SeekTo(p), TimelineMessage::Hover(p)=>Message::TimelineHover(p), TimelineMessage::DragStart=>Message::TimelineSeekStart, TimelineMessage::DragEnd(p)=>Message::TimelineSeekEnd(p) });
        column![
            container(self.video_display.view().map(|m| match m { VideoDisplayMessage::DoubleClick=>Message::ToggleFullscreen, VideoDisplayMessage::RightClick=>Message::ShowSettings(true)}))
                .width(Length::Fill).height(Length::FillPortion(1)).style(|t: &Theme| container::Style{ background: Some(Background::Color(Color::from_rgb(0.05,0.05,0.08))), border: Border::default(), shadow: Shadow::default(), text_color: Some(t.palette().text), snap:false}),
            timeline,
            controls,
        ].spacing(8).width(Length::Fill).height(Length::Fill).into()
    }
}
impl Default for PlayerScreen { fn default() -> Self { Self::new() } }