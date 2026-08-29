//! Google Gemini API client

use std::sync::Arc;
use std::time::Duration;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{warn};
use otip_core::domain::{GridScanRequest, GridScanResponse};
use otip_core::error::{Result, OtipError, ScannerError};
use base64::{Engine as _, engine::general_purpose};

/// Supported Gemini models
pub const GEMINI_37_FLASH: &str = "gemini-3.7-flash";
pub const GEMINI_35_FLASH_LITE: &str = "gemini-3.5-flash-lite";
/// Default and available models - primary: 3.7 Flash, plus 3.5 Flash Lite
pub const AVAILABLE_MODELS: &[&str] = &[GEMINI_37_FLASH, GEMINI_35_FLASH_LITE];
pub const DEFAULT_MODEL: &str = GEMINI_37_FLASH;

/// Human-readable label for a model id
pub fn model_label(model_id: &str) -> &str {
    match model_id {
        GEMINI_37_FLASH => "Gemini 3.7 Flash",
        GEMINI_35_FLASH_LITE => "Gemini 3.5 Flash Lite",
        "gemini-1.5-flash-latest" => "Gemini 1.5 Flash (legacy)",
        "gemini-2.0-flash" => "Gemini 2.0 Flash",
        _ => model_id,
    }
}

/// Configuration for Gemini API
#[derive(Debug, Clone)]
pub struct GeminiConfig {
    pub api_key: String,
    pub model: String,
    pub endpoint: String,
    pub timeout: Duration,
    pub max_retries: u32,
    pub temperature: f32,
    pub max_output_tokens: u32,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: DEFAULT_MODEL.to_string(),
            endpoint: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
            timeout: Duration::from_secs(30),
            max_retries: 3,
            temperature: 0.1,
            max_output_tokens: 100,
        }
    }
}

/// Request/Response structures for Gemini API
#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum GeminiPart {
    Text { text: String },
    InlineData { inline_data: GeminiInlineData },
}

#[derive(Debug, Serialize)]
struct GeminiInlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    temperature: f32,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
    #[serde(rename = "responseMimeType")]
    response_mime_type: String,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: GeminiResponseContent,
}

#[derive(Debug, Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GeminiResponsePart {
    Text { text: String },
    Other(serde_json::Value),
}

/// Gemini API client
pub struct GeminiClient {
    config: GeminiConfig,
    client: Client,
    stats: Arc<RwLock<GeminiStats>>,
}

#[derive(Debug, Default, Clone)]
pub struct GeminiStats {
    pub requests_sent: u64,
    pub successful_responses: u64,
    pub failed_responses: u64,
    pub rate_limited: u64,
    pub total_latency_ms: u64,
}

impl GeminiClient {
    pub fn new(config: GeminiConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| OtipError::Network(e))?;

        Ok(Self {
            config,
            client,
            stats: Arc::new(RwLock::new(GeminiStats::default())),
        })
    }

    /// Analyze a grid image for NSFW content
    pub async fn analyze_grid(&self, request: &GridScanRequest) -> Result<GridScanResponse> {
        if self.config.api_key.is_empty() {
            return Err(OtipError::Scanner(ScannerError::ApiKeyMissing));
        }

        let start = std::time::Instant::now();
        let url = format!(
            "{}/{}:generateContent?key={}",
            self.config.endpoint, self.config.model, self.config.api_key
        );

        // Encode image to base64
        let base64_image = general_purpose::STANDARD.encode(&request.frame_data);

        let prompt = "Analyze this 2x2 grid of video frames (4 seconds total). Each quadrant represents 1 second: top-left=1st second, top-right=2nd second, bottom-left=3rd second, bottom-right=4th second. Identify which quadrants contain explicit NSFW content (nudity, sexual acts, graphic violence). Return ONLY a JSON array of quadrant numbers (1-4) that are explicit. Example: [1, 3] means top-left and bottom-left are explicit. If none are explicit, return [].";

        let gemini_request = GeminiRequest {
            contents: vec![GeminiContent {
                parts: vec![
                    GeminiPart::Text { text: prompt.to_string() },
                    GeminiPart::InlineData { 
                        inline_data: GeminiInlineData {
                            mime_type: request.mime_type.clone(),
                            data: base64_image,
                        }
                    },
                ],
            }],
            generation_config: GeminiGenerationConfig {
                temperature: self.config.temperature,
                max_output_tokens: self.config.max_output_tokens,
                response_mime_type: "application/json".to_string(),
            },
        };

        let mut last_error = None;
        
        for attempt in 0..=self.config.max_retries {
            let response = self.client
                .post(&url)
                .json(&gemini_request)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    let latency = start.elapsed().as_millis() as u64;
                    
                    if status.is_success() {
                        let json: GeminiResponse = resp.json().await
                            .map_err(|e| OtipError::Scanner(ScannerError::ResponseParseError(e.to_string())))?;
                        
                        let explicit_quadrants = self.parse_response(&json)?;
                        
                        let mut stats = self.stats.write().await;
                        stats.requests_sent += 1;
                        stats.successful_responses += 1;
                        stats.total_latency_ms += latency;

                        let confidence_scores = vec![0.9; explicit_quadrants.len()]; // Placeholder
                        
                        return Ok(GridScanResponse {
                            video_id: request.video_id,
                            grid_index: request.grid_index,
                            explicit_quadrants,
                            confidence_scores,
                            processed_at: chrono::Utc::now(),
                        });
                    } else if status == 429 {
                        // Rate limited
                        let retry_after = resp.headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(60);
                        
                        let mut stats = self.stats.write().await;
                        stats.rate_limited += 1;
                        
                        warn!("Rate limited, waiting {}s (attempt {}/{})", retry_after, attempt + 1, self.config.max_retries + 1);
                        tokio::time::sleep(Duration::from_secs(retry_after)).await;
                        continue;
                    } else {
                        let error_text = resp.text().await.unwrap_or_default();
                        last_error = Some(format!("Status {}: {}", status, error_text));
                        
                        if attempt < self.config.max_retries {
                            tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
                            continue;
                        }
                    }
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    if attempt < self.config.max_retries {
                        tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
                        continue;
                    }
                }
            }
        }

        let mut stats = self.stats.write().await;
        stats.requests_sent += 1;
        stats.failed_responses += 1;

        Err(OtipError::Scanner(ScannerError::ApiRequestFailed(
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        )))
    }

    /// Parse Gemini response to extract quadrant numbers
    fn parse_response(&self, response: &GeminiResponse) -> Result<Vec<u8>> {
        let text = response.candidates
            .first()
            .and_then(|c| c.content.parts.first())
            .and_then(|p| match p {
                GeminiResponsePart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .ok_or_else(|| OtipError::Scanner(ScannerError::ResponseParseError(
                "No text in response".to_string()
            )))?;

        // Parse JSON array
        let quadrants: Vec<u8> = serde_json::from_str(text.trim())
            .map_err(|e| OtipError::Scanner(ScannerError::ResponseParseError(e.to_string())))?;

        // Validate quadrant numbers (1-4)
        let valid: Vec<u8> = quadrants.into_iter().filter(|&q| q >= 1 && q <= 4).collect();
        Ok(valid)
    }

    /// Get client statistics
    pub async fn get_stats(&self) -> GeminiStats {
        self.stats.read().await.clone()
    }

    /// Check if API key is configured
    pub fn has_api_key(&self) -> bool {
        !self.config.api_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = GeminiConfig::default();
        assert_eq!(config.model, DEFAULT_MODEL);
        assert_eq!(config.temperature, 0.1);
    }

    #[test]
    fn test_available_models_contains_requested() {
        assert!(AVAILABLE_MODELS.contains(&GEMINI_37_FLASH));
        assert!(AVAILABLE_MODELS.contains(&GEMINI_35_FLASH_LITE));
    }

    #[test]
    fn test_parse_response() {
        let client = GeminiClient {
            config: GeminiConfig::default(),
            client: Client::new(),
            stats: Arc::new(RwLock::new(GeminiStats::default())),
        };

        let response = GeminiResponse {
            candidates: vec![GeminiCandidate {
                content: GeminiResponseContent {
                    parts: vec![GeminiResponsePart::Text { text: "[1, 3]".to_string() }],
                },
            }],
        };

        let result = client.parse_response(&response).unwrap();
        assert_eq!(result, vec![1, 3]);
    }

    #[test]
    fn test_parse_empty_response() {
        let client = GeminiClient {
            config: GeminiConfig::default(),
            client: Client::new(),
            stats: Arc::new(RwLock::new(GeminiStats::default())),
        };

        let response = GeminiResponse {
            candidates: vec![GeminiCandidate {
                content: GeminiResponseContent {
                    parts: vec![GeminiResponsePart::Text { text: "[]".to_string() }],
                },
            }],
        };

        let result = client.parse_response(&response).unwrap();
        let expected: Vec<u8> = vec![];
        assert_eq!(result, expected);
    }
}