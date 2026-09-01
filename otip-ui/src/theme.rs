//! Custom theme for Otip

use iced::{
    Color, Background, Border, Shadow,
    widget::{button, container, slider, pick_list, text_input, scrollable},
    theme::Theme as IcedTheme,
};

/// Custom color palette
pub struct OtipPalette;

impl OtipPalette {
    // Primary colors
    pub const PRIMARY: Color = Color::from_rgb(0.2, 0.6, 0.9);
    pub const PRIMARY_HOVER: Color = Color::from_rgb(0.15, 0.5, 0.8);
    pub const PRIMARY_PRESSED: Color = Color::from_rgb(0.1, 0.4, 0.7);
    
    // Surface colors
    pub const BACKGROUND: Color = Color::from_rgb(0.08, 0.08, 0.1);
    pub const SURFACE: Color = Color::from_rgb(0.12, 0.12, 0.15);
    pub const SURFACE_VARIANT: Color = Color::from_rgb(0.16, 0.16, 0.2);
    pub const SURFACE_ELEVATED: Color = Color::from_rgb(0.2, 0.2, 0.25);
    
    // Text colors
    pub const ON_BACKGROUND: Color = Color::from_rgb(0.95, 0.95, 0.95);
    pub const ON_SURFACE: Color = Color::from_rgb(0.9, 0.9, 0.9);
    pub const ON_SURFACE_VARIANT: Color = Color::from_rgb(0.7, 0.7, 0.75);
    pub const ON_PRIMARY: Color = Color::WHITE;
    
    // Status colors
    pub const SUCCESS: Color = Color::from_rgb(0.0, 0.7, 0.3);
    pub const WARNING: Color = Color::from_rgb(0.95, 0.65, 0.0);
    pub const ERROR: Color = Color::from_rgb(0.9, 0.2, 0.2);
    pub const INFO: Color = Color::from_rgb(0.2, 0.6, 0.9);
    
    // Timeline colors
    pub const TIMELINE_UNKNOWN: Color = Color::from_rgb(0.4, 0.4, 0.45);
    pub const TIMELINE_SAFE: Color = Color::from_rgb(0.0, 0.7, 0.3);
    pub const TIMELINE_EXPLICIT: Color = Color::from_rgb(0.9, 0.2, 0.2);
    pub const TIMELINE_SKIP: Color = Color::from_rgb(0.95, 0.55, 0.0);
    pub const TIMELINE_PLAYHEAD: Color = Color::from_rgb(1.0, 1.0, 1.0);
    pub const TIMELINE_HOVER: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.3);
    
    // Border
    pub const BORDER: Color = Color::from_rgba(0.3, 0.3, 0.35, 0.5);
    pub const BORDER_FOCUS: Color = Color::from_rgb(0.2, 0.6, 0.9);
}

/// Create the custom Otip theme
pub fn otip_theme() -> IcedTheme {
    IcedTheme::custom(
        "Otip Dark".to_string(),
        iced::theme::Palette {
            background: OtipPalette::BACKGROUND,
            text: OtipPalette::ON_BACKGROUND,
            primary: OtipPalette::PRIMARY,
            success: OtipPalette::SUCCESS,
            danger: OtipPalette::ERROR,
            warning: OtipPalette::WARNING,
        },
    )
}

/// Light theme variant
pub fn otip_light_theme() -> IcedTheme {
    IcedTheme::custom(
        "Otip Light".to_string(),
        iced::theme::Palette {
            background: Color::from_rgb(0.96, 0.96, 0.98),
            text: Color::from_rgb(0.1, 0.1, 0.15),
            primary: OtipPalette::PRIMARY,
            success: OtipPalette::SUCCESS,
            danger: OtipPalette::ERROR,
            warning: OtipPalette::WARNING,
        },
    )
}

/// Get theme based on preference
pub fn get_theme(theme_pref: otip_core::domain::Theme) -> IcedTheme {
    match theme_pref {
        otip_core::domain::Theme::Light => otip_light_theme(),
        otip_core::domain::Theme::Dark => otip_theme(),
        otip_core::domain::Theme::System => {
            // In a real app, detect system theme
            otip_theme()
        }
    }
}
