//! Simple timeline widget (placeholder for Iced 0.14)

use std::time::Duration;
use iced::{
    widget::{container, row, text, Space},
    Alignment, Element, Length, Color, Border, Background, Shadow, Theme,
};
use otip_core::timeline::{Timeline, TimelineSegmentType, format_duration_short};

/// Timeline widget for displaying video progress with scan status
pub struct TimelineWidget;

impl Default for TimelineWidget {
    fn default() -> Self { Self::new() }
}
impl TimelineWidget {
    pub fn new() -> Self { Self }
    pub fn clear_cache(&mut self) {}
    pub fn view<'a>(
        &'a self,
        timeline: Option<&'a Timeline>,
        position: Duration,
        duration: Duration,
        hover_position: Option<Duration>,
        _is_dragging: bool,
    ) -> Element<'a, TimelineMessage> {
        let progress = if duration.as_secs_f32() > 0.0 { (position.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0) } else { 0.0 };
        let bar = container(Space::new().width(Length::Fill).height(Length::Fixed(6.0)))
            .width(Length::Fill).height(Length::Fixed(6.0))
            .style(|_: &Theme| container::Style { background: Some(Background::Color(Color::from_rgb(0.2, 0.2, 0.25))), border: Border::default(), shadow: Shadow::default(), text_color: None, snap: false });
        let filled = container(Space::new().width(Length::FillPortion((progress*100.0) as u16)).height(Length::Fill))
            .width(Length::FillPortion((progress*100.0) as u16)).height(Length::Fill)
            .style(|_: &Theme| container::Style { background: Some(Background::Color(Color::from_rgb(0.2, 0.6, 0.9))), border: Border::default(), shadow: Shadow::default(), text_color: None, snap: false });
        let track = iced::widget::stack![bar, filled].width(Length::Fill).height(Length::Fixed(6.0));
        let legend = row![
            legend_item("Gray: Unknown", Color::from_rgb(0.4, 0.4, 0.45)),
            legend_item("Green: Safe", Color::from_rgb(0.0, 0.7, 0.3)),
            legend_item("Red: Explicit", Color::from_rgb(0.9, 0.2, 0.2)),
        ].spacing(12);
        let times = row![
            text(format_duration_short(position)).size(11).color(Color::from_rgb(0.7,0.7,0.75)),
            Space::new().width(Length::Fill),
            text(format_duration_short(duration)).size(11).color(Color::from_rgb(0.7,0.7,0.75)),
        ].width(Length::Fill);
        iced::widget::column![track, Space::new().height(Length::Fixed(4.0)), times, legend].spacing(4).into()
    }
}
fn legend_item<'a>(label: &'a str, color: Color) -> Element<'a, TimelineMessage> {
    row![
        container(Space::new().width(Length::Fixed(12.0)).height(Length::Fixed(12.0))).width(Length::Fixed(12.0)).height(Length::Fixed(12.0)).style(move |_: &Theme| container::Style{ background: Some(Background::Color(color)), border: Border{ radius: 2.0.into(), ..Default::default()}, ..Default::default()}),
        text(label).size(10).color(Color::from_rgb(0.6,0.6,0.65))
    ].spacing(4).align_y(Alignment::Center).into()
}
#[derive(Debug, Clone)]
pub enum TimelineMessage { Seek(Duration), Hover(Option<Duration>), DragStart, DragEnd(Duration) }
