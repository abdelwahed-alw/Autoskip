//! Video display widget - simplified for Iced 0.14

use iced::{
    widget::{container, column, text, Space},
    Alignment, Element, Length, Color, Background, Border, Shadow, Theme,
};
use image::DynamicImage;
use bytes::Bytes;

/// Video display widget for rendering video frames
pub struct VideoDisplay {
    current_frame: Option<DynamicImage>,
    frame_bytes: Option<Bytes>,
    frame_size: (u32, u32),
    show_placeholder: bool,
}
impl Default for VideoDisplay { fn default() -> Self { Self::new() } }
impl VideoDisplay {
    pub fn new() -> Self { Self { current_frame: None, frame_bytes: None, frame_size: (0,0), show_placeholder: true } }
    pub fn update_frame(&mut self, frame: DynamicImage) {
        self.frame_size = (frame.width(), frame.height());
        self.current_frame = Some(frame);
        self.show_placeholder = false;
    }
    pub fn update_frame_bytes(&mut self, bytes: Bytes, width: u32, height: u32) {
        self.frame_bytes = Some(bytes);
        self.frame_size = (width, height);
        self.show_placeholder = false;
    }
    pub fn clear(&mut self) { self.current_frame = None; self.frame_bytes = None; self.frame_size=(0,0); self.show_placeholder=true; }
    pub fn view(&mut self) -> Element<VideoDisplayMessage> {
        let content = if self.show_placeholder {
            column![
                text("🎬").size(64).color(Color::from_rgb(0.2,0.6,0.9)),
                Space::new().height(Length::Fixed(16.0)),
                text("Drop video file or click to open").size(14).color(Color::from_rgb(0.6,0.6,0.65)).align_x(Alignment::Center),
                Space::new().height(Length::Fixed(8.0)),
                text("(MPV stub - video rendering disabled for build)").size(11).color(Color::from_rgb(0.5,0.5,0.55))
            ].align_x(Alignment::Center).spacing(4)
        } else {
            column![
                text("▶ Video Frame").size(18).color(Color::WHITE),
                text(format!("{}x{} preview", self.frame_size.0, self.frame_size.1)).size(12).color(Color::from_rgb(0.7,0.7,0.75))
            ].align_x(Alignment::Center)
        };
        container(content).width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill)
            .style(|theme: &Theme| container::Style{ background: Some(Background::Color(Color::from_rgb(0.05,0.05,0.08))), border: Border::default(), shadow: Shadow::default(), text_color: Some(theme.palette().text), snap: false }).into()
    }
}
#[derive(Debug, Clone)]
pub enum VideoDisplayMessage { DoubleClick, RightClick }