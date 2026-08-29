//! Settings screen - Iced 0.14 with Gemini API key + model selection
use iced::{
    widget::{button, column, container, row, text, text_input, pick_list, Space, scrollable},
    Alignment, Element, Length, Theme, Color, Border, Background, Shadow,
};
use otip_core::domain::{PlaybackMode, Theme as AppTheme};
use otip_core::config::{GEMINI_AVAILABLE_MODELS, gemini_model_label};
use crate::state::{AppState, Message};

/// Wrapper for Gemini model pick_list to provide nice labels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiModelOpt {
    Flash37,
    FlashLite35,
}

impl GeminiModelOpt {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Flash37 => "gemini-3.7-flash",
            Self::FlashLite35 => "gemini-3.5-flash-lite",
        }
    }
    pub fn from_id(id: &str) -> Self {
        match id {
            "gemini-3.5-flash-lite" => Self::FlashLite35,
            // default to 3.7 for any unknown including gemini-1.5 legacy
            _ => Self::Flash37,
        }
    }
    pub fn all() -> [Self; 2] {
        [Self::Flash37, Self::FlashLite35]
    }
}

impl std::fmt::Display for GeminiModelOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flash37 => write!(f, "Gemini 3.7 Flash"),
            Self::FlashLite35 => write!(f, "Gemini 3.5 Flash Lite"),
        }
    }
}

pub struct SettingsScreen;
impl SettingsScreen {
    pub fn view(state: &AppState) -> Element<Message> {
        // Title bar
        let title_bar = row![
            text("Settings").size(26).color(Color::from_rgb(0.95, 0.95, 0.95)),
            Space::new().width(Length::Fill),
            button(text("✕").size(16))
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.16, 0.16, 0.2))),
                    border: Border { color: Color::from_rgb(0.3, 0.3, 0.35), width: 1.0, radius: 6.0.into() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false,
                })
                .on_press(Message::ShowSettings(false))
                .padding([6, 12]),
        ]
        .align_y(Alignment::Center);

        // Playback section
        let playback_section = column![
            text("Playback").size(16).color(Color::from_rgb(0.9, 0.9, 0.95)),
            Space::new().height(Length::Fixed(6.0)),
            text("Default Playback Mode").size(13).color(Color::from_rgb(0.7, 0.7, 0.75)),
            pick_list(
                &[PlaybackMode::SafeMode, PlaybackMode::InstantPlay][..],
                Some(state.preferences.default_playback_mode),
                |m| Message::PreferencesChanged({
                    let mut p = state.preferences.clone();
                    p.default_playback_mode = m;
                    p
                })
            )
            .placeholder("Select mode")
            .width(Length::Fixed(260.0))
            .text_size(14),
        ]
        .spacing(6);

        // Theme section
        let theme_section = column![
            text("Appearance").size(16).color(Color::from_rgb(0.9, 0.9, 0.95)),
            Space::new().height(Length::Fixed(6.0)),
            text("Theme").size(13).color(Color::from_rgb(0.7, 0.7, 0.75)),
            pick_list(
                &[AppTheme::System, AppTheme::Light, AppTheme::Dark][..],
                Some(state.preferences.theme),
                |t| Message::PreferencesChanged({
                    let mut p = state.preferences.clone();
                    p.theme = t;
                    p
                })
            )
            .placeholder("Select theme")
            .width(Length::Fixed(180.0))
            .text_size(14),
        ]
        .spacing(6);

        // Gemini AI section - API key + model
        let current_model_opt = GeminiModelOpt::from_id(&state.gemini_model);
        let api_key_status = if state.gemini_api_key.is_empty() {
            text("⚠ No API key set — scanning will fail").size(12).color(Color::from_rgb(0.95, 0.65, 0.0))
        } else {
            text(format!("✓ API key set ({}...)", if state.gemini_api_key.len() > 8 { &state.gemini_api_key[..4] } else { "****" }))
                .size(12)
                .color(Color::from_rgb(0.0, 0.7, 0.3))
        };

        let gemini_section = column![
            text("AI Content Moderation (Gemini)").size(16).color(Color::from_rgb(0.9, 0.9, 0.95)),
            text("Uses Gemini to scan 2×2 grid thumbnails (4 seconds per request)").size(12).color(Color::from_rgb(0.6, 0.6, 0.65)),
            Space::new().height(Length::Fixed(8.0)),
            // API Key input
            text("Gemini API Key").size(13).color(Color::from_rgb(0.8, 0.8, 0.85)),
            row![
                text_input("paste your Gemini API key (AIza...) ", &state.gemini_api_key)
                    .on_input(Message::ApiKeyChanged)
                    .secure(!state.gemini_api_key_visible)
                    .padding(10)
                    .size(13)
                    .width(Length::Fill),
                button(text(if state.gemini_api_key_visible { "🙈 Hide" } else { "👁 Show" }).size(12))
                    .style(|_: &Theme, _| button::Style {
                        background: Some(Background::Color(Color::from_rgb(0.16, 0.16, 0.2))),
                        border: Border { color: Color::from_rgb(0.3, 0.3, 0.35), width: 1.0, radius: 6.0.into() },
                        text_color: Color::WHITE,
                        shadow: Shadow::default(),
                        snap: false,
                    })
                    .on_press(Message::ApiKeyVisibilityToggled)
                    .padding([8, 10]),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            api_key_status,
            text("Get a key at aistudio.google.com/app/apikey  • stored in config.toml and $GEMINI_API_KEY env fallback").size(11).color(Color::from_rgb(0.5, 0.5, 0.55)),
            Space::new().height(Length::Fixed(10.0)),
            // Model picker
            text("Model").size(13).color(Color::from_rgb(0.8, 0.8, 0.85)),
            pick_list(
                GeminiModelOpt::all(),
                Some(current_model_opt),
                |opt| Message::GeminiModelChanged(opt.id().to_string())
            )
            .placeholder("Select Gemini model")
            .width(Length::Fixed(260.0))
            .text_size(14),
            text(format!("Selected: {}  ({})", gemini_model_label(&state.gemini_model), state.gemini_model)).size(11).color(Color::from_rgb(0.6, 0.6, 0.65)),
            // Also show raw available ids for debugging
            text(format!("Available: {}", GEMINI_AVAILABLE_MODELS.join(", "))).size(10).color(Color::from_rgb(0.5, 0.5, 0.55)),
        ]
        .spacing(6);

        let save_row = row![
            button(text("Save Gemini Settings").size(14))
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.2, 0.6, 0.9))),
                    border: Border { color: Color::TRANSPARENT, width: 0.0, radius: 8.0.into() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false,
                })
                .on_press(Message::SaveGeminiConfig)
                .padding([10, 18]),
            button(text("Close").size(14))
                .style(|_: &Theme, _| button::Style {
                    background: Some(Background::Color(Color::from_rgb(0.16, 0.16, 0.2))),
                    border: Border { color: Color::from_rgb(0.3, 0.3, 0.35), width: 1.0, radius: 8.0.into() },
                    text_color: Color::WHITE,
                    shadow: Shadow::default(),
                    snap: false,
                })
                .on_press(Message::ShowSettings(false))
                .padding([10, 18]),
        ]
        .spacing(12);

        let content = column![
            title_bar,
            Space::new().height(Length::Fixed(16.0)),
            playback_section,
            Space::new().height(Length::Fixed(14.0)),
            theme_section,
            Space::new().height(Length::Fixed(14.0)),
            // Divider
            container(Space::new().height(Length::Fixed(1.0)).width(Length::Fill))
                .style(|_: &Theme| container::Style {
                    background: Some(Background::Color(Color::from_rgba(0.3, 0.3, 0.35, 0.5))),
                    border: Border::default(),
                    shadow: Shadow::default(),
                    text_color: None,
                    snap: false,
                }),
            Space::new().height(Length::Fixed(14.0)),
            gemini_section,
            Space::new().height(Length::Fixed(18.0)),
            save_row,
            Space::new().height(Length::Fixed(8.0)),
            text("Settings are saved to config.toml (and applied on next scan)").size(11).color(Color::from_rgb(0.5, 0.5, 0.55)),
        ]
        .spacing(4)
        .width(Length::Fill)
        .max_width(560.0);

        let inner = container(content)
            .padding(22)
            .width(Length::Fill)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(Color::from_rgb(0.12, 0.12, 0.15))),
                border: Border { color: Color::from_rgb(0.25, 0.25, 0.3), width: 1.0, radius: 12.0.into() },
                shadow: Shadow::default(),
                text_color: Some(Color::from_rgb(0.95, 0.95, 0.95)),
                snap: false,
            });

        // Make scrollable for small windows
        let scroll = scrollable(inner)
            .width(Length::Fill)
            .height(Length::Fill);

        container(scroll)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
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
}
