//! Error types for the application

use std::fmt;
use thiserror::Error;

/// Main error type for the application
#[derive(Error, Debug)]
pub enum OtipError {
    #[error("Video error: {0}")]
    Video(#[from] VideoError),

    #[error("AI/Scanner error: {0}")]
    Scanner(#[from] ScannerError),

    #[error("UI error: {0}")]
    Ui(#[from] UiError),

    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Image processing error: {0}")]
    Image(#[from] image::ImageError),

    #[error("Channel error: {0}")]
    Channel(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Cancelled")]
    Cancelled,

    #[error("Other: {0}")]
    Other(String),
}

#[derive(Error, Debug)]
pub enum VideoError {
    #[error("Failed to initialize video engine: {0}")]
    InitFailed(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Decode error: {0}")]
    DecodeError(String),

    #[error("Seek failed: {0}")]
    SeekFailed(String),

    #[error("No video track found")]
    NoVideoTrack,

    #[error("Hardware acceleration not available: {0}")]
    HwAccelUnavailable(String),

    #[error("MPV error: {0}")]
    Mpv(String),

    #[error("GStreamer error: {0}")]
    GStreamer(String),
}

#[derive(Error, Debug)]
pub enum ScannerError {
    #[error("API key not configured")]
    ApiKeyMissing,

    #[error("API request failed: {0}")]
    ApiRequestFailed(String),

    #[error("API response parsing failed: {0}")]
    ResponseParseError(String),

    #[error("Rate limited: retry after {0} seconds")]
    RateLimited(u64),

    #[error("Frame extraction failed: {0}")]
    FrameExtractionFailed(String),

    #[error("Grid creation failed: {0}")]
    GridCreationFailed(String),

    #[error("Scan cancelled")]
    Cancelled,

    #[error("Video too short for scanning")]
    VideoTooShort,

    #[error("Confidence below threshold")]
    LowConfidence,
}

#[derive(Error, Debug)]
pub enum UiError {
    #[error("Window creation failed: {0}")]
    WindowCreationFailed(String),

    #[error("Renderer error: {0}")]
    RendererError(String),

    #[error("Font loading failed: {0}")]
    FontLoadFailed(String),

    #[error("Theme error: {0}")]
    ThemeError(String),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Config file not found: {0}")]
    NotFound(String),

    #[error("Config parse error: {0}")]
    ParseError(String),

    #[error("Config validation failed: {0}")]
    ValidationError(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

pub type Result<T> = std::result::Result<T, OtipError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let otip_err: OtipError = io_err.into();
        assert!(matches!(otip_err, OtipError::Io(_)));
    }
}