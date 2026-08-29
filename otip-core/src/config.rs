//! Configuration management

use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use directories::ProjectDirs;
use crate::domain::{UserPreferences, Theme, PlaybackMode};

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub preferences: UserPreferences,
    pub gemini_api_key: Option<String>,
    pub gemini_model: String,
    pub gemini_endpoint: String,
    pub scan_frame_interval: u32, // seconds between frames
    pub grid_size: (u32, u32),    // frames per grid (2x2 = 4 frames)
    pub frame_resolution: (u32, u32), // extraction resolution
    pub max_concurrent_scans: usize,
    pub cache_dir: PathBuf,
    pub log_level: String,
}

/// Supported Gemini models exposed to UI - mirrors otip-ai
pub const GEMINI_37_FLASH: &str = "gemini-3.7-flash";
pub const GEMINI_35_FLASH_LITE: &str = "gemini-3.5-flash-lite";
pub const GEMINI_AVAILABLE_MODELS: &[&str] = &[GEMINI_37_FLASH, GEMINI_35_FLASH_LITE];
pub const GEMINI_DEFAULT_MODEL: &str = GEMINI_37_FLASH;

pub fn gemini_model_label(model_id: &str) -> &str {
    match model_id {
        GEMINI_37_FLASH => "Gemini 3.7 Flash",
        GEMINI_35_FLASH_LITE => "Gemini 3.5 Flash Lite",
        "gemini-1.5-flash-latest" => "Gemini 1.5 Flash (legacy)",
        "gemini-2.0-flash" => "Gemini 2.0 Flash",
        _ => model_id,
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let cache_dir = ProjectDirs::from("com", "otip", "otip")
            .map(|dirs: ProjectDirs| dirs.cache_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/tmp/otip"));

        Self {
            preferences: UserPreferences::default(),
            gemini_api_key: None,
            gemini_model: GEMINI_DEFAULT_MODEL.to_string(),
            gemini_endpoint: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
            scan_frame_interval: 1, // 1 frame per second
            grid_size: (2, 2),
            frame_resolution: (320, 240),
            max_concurrent_scans: 3,
            cache_dir,
            log_level: "info".to_string(),
        }
    }
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let config_dir = ProjectDirs::from("com", "otip", "otip")
            .map(|dirs: ProjectDirs| dirs.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        
        let config_path = config_dir.join("config.toml");
        
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: AppConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            let config = Self::default();
            config.save()?;
            Ok(config)
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let config_dir = ProjectDirs::from("com", "otip", "otip")
            .map(|dirs: ProjectDirs| dirs.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("config.toml");
        let content = toml::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    pub fn get_gemini_api_key(&self) -> Option<String> {
        self.gemini_api_key.clone()
            .or_else(|| self.preferences.api_key.clone())
            .or_else(|| std::env::var("GEMINI_API_KEY").ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.scan_frame_interval, 1);
        assert_eq!(config.grid_size, (2, 2));
        assert_eq!(config.frame_resolution, (320, 240));
    }
}