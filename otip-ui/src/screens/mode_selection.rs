//! Pre-play mode selection screen - minimal 0.14

use iced::{widget::{button, column, container, row, text, Space}, Alignment, Element, Length, Theme, Color, Border, Background, Shadow};
use otip_core::domain::PlaybackMode;
use crate::state::Message;
pub struct ModeSelectionScreen { selected_mode: PlaybackMode }
impl ModeSelectionScreen {
    pub fn new(m: PlaybackMode) -> Self { Self { selected_mode: m } }
    pub fn view(&self, video_name: &str) -> Element<Message> {
        let content = column![
            text("Content Moderation").size(28).color(Color::from_rgb(0.95,0.95,0.95)),
            text(format!("Ready to play: {}", video_name)).size(14).color(Color::from_rgb(0.7,0.7,0.75)),
            Space::new().height(Length::Fixed(16.0)),
            row![
                mode_card(PlaybackMode::SafeMode, self.selected_mode==PlaybackMode::SafeMode, "Safe Mode"),
                mode_card(PlaybackMode::InstantPlay, self.selected_mode==PlaybackMode::InstantPlay, "Instant Play"),
            ].spacing(12),
            Space::new().height(Length::Fixed(16.0)),
            row![
                button(text("Cancel")).style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgb(0.12,0.12,0.15))), border: Border{color: Color::from_rgb(0.3,0.3,0.35), width:1.0, radius:8.0.into()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }).on_press(Message::CancelMode).width(Length::FillPortion(1)),
                button(text("Continue")).style(|_: &Theme, _| button::Style{ background: Some(Background::Color(Color::from_rgb(0.2,0.6,0.9))), border: Border{color: Color::TRANSPARENT, width:0.0, radius:8.0.into()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false }).on_press(Message::ConfirmMode).width(Length::FillPortion(2)),
            ].spacing(12)
        ].spacing(12).align_x(Alignment::Center);
        container(content).padding(20).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill)
            .style(|t: &Theme| container::Style{ background: Some(Background::Color(t.palette().background)), border: Border::default(), shadow: Shadow::default(), text_color: Some(t.palette().text), snap:false }).into()
    }
    pub fn selected_mode(&self) -> PlaybackMode { self.selected_mode }
    pub fn set_selected_mode(&mut self, m: PlaybackMode) { self.selected_mode=m; }
}
fn mode_card<'a>(mode: PlaybackMode, sel: bool, title: &'a str) -> Element<'a, Message> {
    button(container(text(title).size(16)).padding(12).width(Length::Fill).center_x(Length::Fill))
        .style(move |_: &Theme, _| button::Style{ background: Some(Background::Color(if sel {Color::from_rgba(0.2,0.6,0.9,0.2)} else {Color::from_rgb(0.12,0.12,0.15)})), border: Border{color: if sel {Color::from_rgb(0.2,0.6,0.9)} else {Color::from_rgb(0.3,0.3,0.35)}, width:1.0, radius:8.0.into()}, text_color: Color::WHITE, shadow: Shadow::default(), snap:false })
        .on_press(Message::ModeSelected(mode)).width(Length::Fill).into()
}