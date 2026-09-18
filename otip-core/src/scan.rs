//! Video scanning logic and grid processing
//! Thumbnail extraction: single frame at 5-second mark via GStreamer, cached in memory / temp file, rendered via iced::widget::image

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn, error};
use image::{DynamicImage, ImageBuffer, Rgb};
use base64::Engine;
use crate::domain::{
    VideoId, ScanSegment, QuadrantFlags, TimelineSegment, TimelineSegmentType,
    GridScanRequest, GridScanResponse, ScanProgress
};
use crate::error::{Result, OtipError, ScannerError};
use crate::config::AppConfig;

/// Max per-frame width sent to Gemini. The AI does not need 1080p to detect
/// a product or a scene — 320px is plenty and avoids 503s from huge payloads.
pub const MAX_FRAME_WIDTH: u32 = 320;
/// Max per-frame height paired with `MAX_FRAME_WIDTH` (preserves 4:3 cells).
pub const MAX_FRAME_HEIGHT: u32 = 240;
/// JPEG quality for the final grid upload (70–80% balances size vs. detail).
pub const GRID_JPEG_QUALITY: u8 = 75;
/// Gemini API timeout — generous enough to ride out congestion without hanging.
pub const GEMINI_TIMEOUT_SECS: u64 = 60;
/// Frame extraction rate for `run_ai_scan`: 1 frame per second.
pub const AI_SCAN_FRAME_INTERVAL: Duration = Duration::from_secs(1);
/// Frames per Gemini grid in `run_ai_scan`: small API-friendly batches.
/// 4 frames = one 2x2 grid; keeps each request tiny so long videos never
/// build a single massive payload (the old 503 cause).
pub const AI_SCAN_FRAMES_PER_BATCH: usize = 4;
/// Hard cap: no batch may ever exceed this many frames. Batches are rendered
/// as one grid image each, so this bounds every single API payload.
pub const AI_SCAN_MAX_FRAMES_PER_BATCH: usize = 20;
/// Streaming phase boundaries (fractions of overall progress):
/// - Phase 1 FFmpeg extraction: 0% → `AI_SCAN_EXTRACTION_END` (30%)
/// - Phase 2 combining/batching: → `AI_SCAN_COMBINING_END` (40%)
/// - Phase 3 sequential AI calls: → `AI_SCAN_ANALYSIS_END` (90%)
/// - Phase 4 merge + cleanup: → 100%
pub const AI_SCAN_EXTRACTION_END: f32 = 0.30;
pub const AI_SCAN_COMBINING_END: f32 = 0.40;
pub const AI_SCAN_ANALYSIS_END: f32 = 0.90;
/// Pacing between sequential Gemini batch calls (free-tier rate protection).
/// 1361 frames → 341 batches back-to-back is exactly what earns 429s.
pub const AI_SCAN_BATCH_DELAY_SECS: u64 = 4;
/// Retries per batch when Gemini answers 429/503 before skipping the batch.
pub const AI_SCAN_MAX_RETRIES: u32 = 3;
/// Base backoff between retries (exponential: 10s, 20s, 40s).
pub const AI_SCAN_RETRY_BASE_DELAY_SECS: u64 = 10;
/// Videos longer than this get sparser extraction (1fps would drown the API).
pub const AI_SCAN_LONG_VIDEO_SECS: u64 = 300; // 5 minutes
/// One streaming progress update: fraction 0.0..=1.0 plus a status label
/// (e.g. `(0.45, "Analyzing batch 1/4...")`).
pub type AiScanProgressUpdate = (f32, String);

/// Best-effort send of one progress update. Never blocks the scan (unbounded
/// channel) and silently drops when the UI is gone.
fn emit_progress(
    tx: &Option<tokio::sync::mpsc::UnboundedSender<AiScanProgressUpdate>>,
    frac: f32,
    label: impl Into<String>,
) {
    if let Some(tx) = tx {
        let _ = tx.send((frac.clamp(0.0, 1.0), label.into()));
    }
}

/// Overall fraction at the START of batch `idx` (0-based) of `count`.
/// Maps Phase 3 sequential AI calls [`AI_SCAN_COMBINING_END`]→[`AI_SCAN_ANALYSIS_END`].
fn batch_progress(idx: usize, count: usize) -> f32 {
    if count == 0 {
        return AI_SCAN_ANALYSIS_END;
    }
    AI_SCAN_COMBINING_END
        + (AI_SCAN_ANALYSIS_END - AI_SCAN_COMBINING_END)
            * ((idx as f32) / (count as f32).max(1.0))
}

/// Overall fraction for FFmpeg extraction progress (Phase 1: 0%→30%).
fn extraction_progress(done_frames: u64, expected_frames: Option<u64>) -> f32 {
    match expected_frames {
        Some(exp) if exp > 0 => {
            AI_SCAN_EXTRACTION_END * ((done_frames as f32) / (exp as f32)).min(1.0)
        }
        // Unknown length: small nonzero pulse so the bar visibly moves.
        _ => 0.1,
    }
}

/// Overall fraction while pre-building batch grids (Phase 2: 30%→40%).
fn combining_progress(done_grids: usize, total_batches: usize) -> f32 {
    if total_batches == 0 {
        return AI_SCAN_COMBINING_END;
    }
    AI_SCAN_EXTRACTION_END
        + (AI_SCAN_COMBINING_END - AI_SCAN_EXTRACTION_END)
            * ((done_grids as f32) / (total_batches as f32).max(1.0))
}

/// Adaptive extraction step in seconds: 1fps for short videos, sparser for
/// long ones so total batch counts (and API calls) stay manageable.
/// - ≤5 min → every 1s (1361-frame blowups can't happen)
/// - 5–15 min → every 2s
/// - >15 min → every 3s
pub fn adaptive_frame_step_secs(duration_secs: Option<u64>) -> u64 {
    match duration_secs {
        Some(d) if d > 900 => 3,
        Some(d) if d > AI_SCAN_LONG_VIDEO_SECS => 2,
        _ => 1,
    }
}

/// Rate-limit signal from Gemini: 429 Too Many Requests or 503 overloaded.
/// Anything else (including other 4xx/5xx) is terminal for that batch.
fn is_retryable_status(code: u16) -> bool {
    code == 429 || code == 503
}

/// Exponential backoff before retry `attempt` (1-based): 10s, 20s, 40s…
fn retry_backoff_secs(attempt: u32) -> u64 {
    AI_SCAN_RETRY_BASE_DELAY_SECS.saturating_mul(
        2u64.saturating_pow(attempt.saturating_sub(1)),
    )
}

/// Count `frame_*.jpg` outputs in `dir` (extraction progress polling).
fn count_frame_jpgs(dir: &std::path::Path) -> u64 {
    let mut n = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("jpg")
                && p.file_name()
                    .and_then(|x| x.to_str())
                    .map(|x| x.starts_with("frame_"))
                    .unwrap_or(false)
            {
                n += 1;
            }
        }
    }
    n
}

/// Probe total duration in whole seconds via ffprobe. `None` when ffprobe is
/// missing or the file is unreadable — callers degrade to pulsed progress.
async fn probe_duration_secs(video_path: &std::path::Path) -> Option<u64> {
    let out = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            &video_path.to_string_lossy(),
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .map(|d| d as u64)
        .filter(|d| *d > 0)
}

/// Scanner configuration
#[derive(Debug, Clone)]
pub struct ScannerConfig {
    pub frame_interval: Duration,
    pub grid_size: (u32, u32),
    pub frame_resolution: (u32, u32),
    pub confidence_threshold: f32,
    pub max_concurrent: usize,
    pub api_key: String,
    pub model: String,
    pub endpoint: String,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            frame_interval: Duration::from_secs(1),
            grid_size: (2, 2),
            frame_resolution: (320, 240),
            confidence_threshold: 0.7,
            max_concurrent: 3,
            api_key: String::new(),
            model: crate::config::GEMINI_DEFAULT_MODEL.to_string(),
            endpoint: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
        }
    }
}

impl From<&AppConfig> for ScannerConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            frame_interval: Duration::from_secs(config.scan_frame_interval as u64),
            grid_size: config.grid_size,
            frame_resolution: config.frame_resolution,
            confidence_threshold: config.preferences.confidence_threshold,
            max_concurrent: config.max_concurrent_scans,
            api_key: config.get_gemini_api_key().unwrap_or_default(),
            model: config.gemini_model.clone(),
            endpoint: config.gemini_endpoint.clone(),
        }
    }
}

/// Statistics for the scanner
#[derive(Debug, Default, Clone)]
pub struct ScannerStats {
    pub frames_processed: u64,
    pub grids_sent: u64,
    pub explicit_found: u64,
    pub api_errors: u64,
    pub total_scan_time: Duration,
}

/// Main scanner struct
pub struct VideoScanner {
    config: ScannerConfig,
    stats: Arc<RwLock<ScannerStats>>,
    progress_tx: mpsc::UnboundedSender<ScanProgress>,
    segment_tx: mpsc::UnboundedSender<(VideoId, TimelineSegment)>,
    client: reqwest::Client,
}

impl VideoScanner {
    pub fn new(
        config: ScannerConfig,
        progress_tx: mpsc::UnboundedSender<ScanProgress>,
        segment_tx: mpsc::UnboundedSender<(VideoId, TimelineSegment)>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(GEMINI_TIMEOUT_SECS))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            stats: Arc::new(RwLock::new(ScannerStats::default())),
            progress_tx,
            segment_tx,
            client,
        }
    }

    /// Start scanning a video
    pub async fn scan_video(
        &self,
        video_id: VideoId,
        video_path: String,
        total_duration: Duration,
        frame_receiver: mpsc::UnboundedReceiver<(Duration, DynamicImage)>,
    ) -> Result<()> {
        info!("Starting scan for video {}", video_id);
        
        let mut scanned_duration = Duration::ZERO;
        let mut segments_found = 0;
        let mut explicit_segments = 0;
        let mut frame_buffer = Vec::new();
        let mut grid_index = 0u32;
        let frames_per_grid = (self.config.grid_size.0 * self.config.grid_size.1) as usize;

        let mut frame_receiver = frame_receiver;
        
        while let Some((timestamp, frame)) = frame_receiver.recv().await {
            // Resize frame to target resolution
            let resized = self.resize_frame(frame)?;
            frame_buffer.push((timestamp, resized));
            scanned_duration = timestamp;

            // Send progress update every 10 frames
            if frame_buffer.len() % 10 == 0 {
                let _ = self.progress_tx.send(ScanProgress {
                    video_id,
                    scanned_duration,
                    total_duration,
                    segments_found,
                    explicit_segments,
                    current_position: timestamp,
                    is_complete: false,
                    error: None,
                });
            }

            // When we have enough frames for a grid, process it
            if frame_buffer.len() >= frames_per_grid {
                let grid_frames: Vec<_> = frame_buffer.drain(..frames_per_grid).collect();
                let grid_start_time = grid_frames[0].0;

                match self.create_grid_image(&grid_frames) {
                    Ok(grid_image) => {
                        let request = GridScanRequest {
                            video_id,
                            grid_index,
                            start_time: grid_start_time,
                            frame_data: grid_image,
                            mime_type: "image/jpeg".to_string(),
                        };

                        if let Err(e) = self.process_grid(request).await {
                            error!("Grid processing failed: {}", e);
                            let mut stats = self.stats.write().await;
                            stats.api_errors += 1;
                        } else {
                            let mut stats = self.stats.write().await;
                            stats.grids_sent += 1;
                        }
                        grid_index += 1;
                    }
                    Err(e) => {
                        error!("Failed to create grid image: {}", e);
                    }
                }

                segments_found += frames_per_grid;
            }
        }

        // Process any remaining frames
        if !frame_buffer.is_empty() {
            let grid_start_time = frame_buffer[0].0;
            if let Ok(grid_image) = self.create_grid_image(&frame_buffer) {
                let request = GridScanRequest {
                    video_id,
                    grid_index,
                    start_time: grid_start_time,
                    frame_data: grid_image,
                    mime_type: "image/jpeg".to_string(),
                };
                let _ = self.process_grid(request).await;
            }
        }

        // Send completion
        let _ = self.progress_tx.send(ScanProgress {
            video_id,
            scanned_duration: total_duration,
            total_duration,
            segments_found,
            explicit_segments,
            current_position: total_duration,
            is_complete: true,
            error: None,
        });

        info!("Scan complete for video {}", video_id);
        Ok(())
    }

    /// Create a 2x2 grid image from 4 frames (downscaled + JPEG-compressed).
    fn create_grid_image(&self, frames: &[(Duration, DynamicImage)]) -> Result<Vec<u8>> {
        let (grid_w, grid_h) = self.config.grid_size;
        // Cap cell size so a 1080p source never blows up the payload —
        // the AI does not need full resolution to detect a scene.
        let (cfg_w, cfg_h) = self.config.frame_resolution;
        let frame_w = cfg_w.min(MAX_FRAME_WIDTH);
        let frame_h = cfg_h.min(MAX_FRAME_HEIGHT);

        let grid_width = grid_w * frame_w;
        let grid_height = grid_h * frame_h;

        let mut grid = ImageBuffer::new(grid_width, grid_height);

        for (idx, (_, frame)) in frames.iter().enumerate() {
            if idx >= (grid_w * grid_h) as usize {
                break;
            }

            let row = (idx as u32) / grid_w;
            let col = (idx as u32) % grid_w;
            let x = col * frame_w;
            let y = row * frame_h;

            // Downscale each 1080p frame to ≤320px wide before compositing.
            // `thumbnail` preserves aspect and is far cheaper than Lanczos3
            // on full-res inputs; `resize_exact` then fits the fixed cell.
            let downscaled = frame.thumbnail(frame_w, frame_h);
            let fitted =
                downscaled.resize_exact(frame_w, frame_h, image::imageops::FilterType::Triangle);
            // Convert to RGB if needed (JPEG has no alpha channel)
            let rgb_frame: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> = fitted.to_rgb8();

            for fy in 0..frame_h.min(rgb_frame.height()) {
                for fx in 0..frame_w.min(rgb_frame.width()) {
                    let pixel = rgb_frame.get_pixel(fx, fy);
                    grid.put_pixel(x + fx, y + fy, *pixel);
                }
            }
        }

        // Compress final grid as JPEG (quality 75) instead of lossless PNG —
        // typically 5–10x smaller Base64 payload, avoids 503s/timeouts.
        let mut bytes = Vec::new();
        {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut bytes,
                GRID_JPEG_QUALITY,
            );
            use image::ImageEncoder as _;
            encoder
                .write_image(
                    &grid.into_raw(),
                    grid_width,
                    grid_height,
                    image::ExtendedColorType::Rgb8,
                )
                .map_err(|e| OtipError::Scanner(ScannerError::GridCreationFailed(e.to_string())))?;
        }

        Ok(bytes)
    }

    /// Resize frame to target resolution, capped at `MAX_FRAME_WIDTH`.
    fn resize_frame(&self, frame: DynamicImage) -> Result<DynamicImage> {
        let (cfg_w, cfg_h) = self.config.frame_resolution;
        let w = cfg_w.min(MAX_FRAME_WIDTH);
        let h = cfg_h.min(MAX_FRAME_HEIGHT);
        // Downscale massive inputs (e.g. 1080p) to ≤320px wide. `thumbnail`
        // preserves aspect ratio, never upscales tiny frames, and is much
        // cheaper than Lanczos3 on full-res sources.
        Ok(frame.thumbnail(w, h))
    }

    /// Send grid to Gemini API
    async fn process_grid(&self, request: GridScanRequest) -> Result<()> {
        if self.config.api_key.is_empty() {
            return Err(OtipError::Scanner(ScannerError::ApiKeyMissing));
        }

        let url = format!(
            "{}/{}:generateContent?key={}",
            self.config.endpoint, self.config.model, self.config.api_key
        );

        let base64_image = base64::engine::general_purpose::STANDARD.encode(&request.frame_data);
        
        let payload = serde_json::json!({
            "contents": [{
                "parts": [
                    {
                        "text": "Analyze this 2x2 grid of video frames (4 seconds total). Each quadrant represents 1 second: top-left=1st second, top-right=2nd second, bottom-left=3rd second, bottom-right=4th second. Identify which quadrants contain explicit NSFW content (nudity, sexual acts, graphic violence). Return ONLY a JSON array of quadrant numbers (1-4) that are explicit. Example: [1, 3] means top-left and bottom-left are explicit. If none are explicit, return []."
                    },
                    {
                        "inline_data": {
                            "mime_type": "image/jpeg",
                            "data": base64_image
                        }
                    }
                ]
            }],
            "generationConfig": {
                "temperature": 0.1,
                "maxOutputTokens": 100,
                "responseMimeType": "application/json"
            }
        });

        let response = self.client
            .post(&url)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(OtipError::Scanner(ScannerError::ApiRequestFailed(
                format!("Status {}: {}", status, text)
            )));
        }

        let json: serde_json::Value = response.json().await?;
        
        // Parse response
        let explicit_quadrants = self.parse_gemini_response(&json)?;
        
        let grid_duration = Duration::from_secs(self.config.grid_size.0 as u64 * self.config.grid_size.1 as u64);
        
        for &quadrant in &explicit_quadrants {
            let segment_start = request.start_time + Duration::from_secs((quadrant - 1) as u64);
            let segment_end = segment_start + Duration::from_secs(1);
            
            let segment = TimelineSegment {
                start_time: segment_start,
                end_time: segment_end,
                segment_type: TimelineSegmentType::ExplicitContent,
                scan_segment: Some(ScanSegment {
                    start_time: segment_start,
                    end_time: segment_end,
                    is_explicit: true,
                    confidence: 0.9, // Would come from response
                    quadrant_flags: Some(QuadrantFlags::from_quadrant_numbers(&[quadrant])),
                }),
            };
            
            let _ = self.segment_tx.send((request.video_id, segment));
            
            let mut stats = self.stats.write().await;
            stats.explicit_found += 1;
        }

        // Also send safe segments for non-flagged quadrants
        for q in 1..=4 {
            if !explicit_quadrants.contains(&q) {
                let segment_start = request.start_time + Duration::from_secs((q - 1) as u64);
                let segment_end = segment_start + Duration::from_secs(1);
                
                let segment = TimelineSegment {
                    start_time: segment_start,
                    end_time: segment_end,
                    segment_type: TimelineSegmentType::ScannedSafe,
                    scan_segment: Some(ScanSegment {
                        start_time: segment_start,
                        end_time: segment_end,
                        is_explicit: false,
                        confidence: 0.9,
                        quadrant_flags: Some(QuadrantFlags::from_quadrant_numbers(&[q])),
                    }),
                };
                
                let _ = self.segment_tx.send((request.video_id, segment));
            }
        }

        let mut stats = self.stats.write().await;
        stats.frames_processed += (self.config.grid_size.0 * self.config.grid_size.1) as u64;

        Ok(())
    }

    /// Parse Gemini API response
    fn parse_gemini_response(&self, json: &serde_json::Value) -> Result<Vec<u8>> {
        // Try to extract the JSON array from the response
        let text = json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .ok_or_else(|| OtipError::Scanner(ScannerError::ResponseParseError(
                "No text in response".to_string()
            )))?;

        // Parse the JSON array
        let quadrants: Vec<u8> = serde_json::from_str(text.trim())
            .map_err(|e| OtipError::Scanner(ScannerError::ResponseParseError(e.to_string())))?;

        // Validate quadrant numbers
        let valid: Vec<u8> = quadrants.into_iter().filter(|&q| q >= 1 && q <= 4).collect();
        Ok(valid)
    }

    pub async fn get_stats(&self) -> ScannerStats {
        self.stats.read().await.clone()
    }
}

/// Frame extractor trait for video engines
#[async_trait::async_trait]
pub trait FrameExtractor: Send + Sync {
    async fn start_extraction(
        &self,
        video_id: VideoId,
        path: &str,
        interval: Duration,
        tx: mpsc::UnboundedSender<(Duration, DynamicImage)>,
    ) -> Result<()>;

    async fn stop_extraction(&self, video_id: VideoId) -> Result<()>;
}

/// Default instruction when the user leaves the custom AI skip prompt empty.
pub const DEFAULT_AI_SKIP_PROMPT: &str =
    "Detect sponsorships, intros/outros, and explicit or sensitive content worth skipping.";

/// Parse a single timestamp token into a `Duration`.
///
/// Accepted forms (whitespace trimmed, trailing `s` allowed):
/// - `SS`, `SS.s` (plain seconds, e.g. `75`, `75.5`, `75s`)
/// - `MM:SS`, `MM:SS.s` (e.g. `01:15`)
/// - `HH:MM:SS` (e.g. `01:02:03`)
pub fn parse_timestamp_token(token: &str) -> Option<Duration> {
    let t = token.trim().trim_end_matches('s').trim();
    if t.is_empty() {
        return None;
    }
    // Strip a leading '[' / '"' / '(' that often surrounds JSON-ish output.
    let t = t.trim_matches(|c| c == '[' || c == ']' || c == '"' || c == '\'' || c == '(' || c == ')');
    if t.is_empty() {
        return None;
    }
    let parts: Vec<&str> = t.split(':').collect();
    match parts.len() {
        1 => parts[0]
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| *v >= 0.0)
            .map(Duration::from_secs_f64),
        2 => {
            let m: f64 = parts[0].trim().parse().ok()?;
            let s: f64 = parts[1].trim().parse().ok()?;
            if m < 0.0 || s < 0.0 {
                return None;
            }
            Some(Duration::from_secs_f64(m * 60.0 + s))
        }
        3 => {
            let h: f64 = parts[0].trim().parse().ok()?;
            let m: f64 = parts[1].trim().parse().ok()?;
            let s: f64 = parts[2].trim().parse().ok()?;
            if h < 0.0 || m < 0.0 || s < 0.0 {
                return None;
            }
            Some(Duration::from_secs_f64(h * 3600.0 + m * 60.0 + s))
        }
        _ => None,
    }
}

/// Parse AI-produced skip-segment output into `(start, end)` durations.
///
/// Handles the shapes Gemini commonly returns:
/// - JSON array of objects: `[{"start": 15, "end": 25}, {"start": "00:30", "end": "00:40"}]`
/// - JSON array of pairs: `[[15, 25]]`
/// - JSON array of strings: `["00:15-00:25"]`
/// - Plain text lines: `00:15-00:25`, `15s - 25s`, `15 --> 25`, `15 to 25`
///
/// Invalid / zero-length ranges are dropped, output is sorted and de-duplicated.
pub fn parse_ai_timestamps(text: &str) -> Vec<(Duration, Duration)> {
    let mut out: Vec<(Duration, Duration)> = Vec::new();

    // 1) Structured JSON attempt first.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        if let Some(arr) = v.as_array() {
            for item in arr {
                match item {
                    serde_json::Value::Object(map) => {
                        let start_v = map.get("start").or_else(|| map.get("start_time"));
                        let end_v = map.get("end").or_else(|| map.get("end_time"));
                        if let (Some(s), Some(e)) = (start_v, end_v) {
                            let s = json_value_to_duration(s);
                            let e = json_value_to_duration(e);
                            if let (Some(s), Some(e)) = (s, e) {
                                if e > s {
                                    out.push((s, e));
                                }
                            }
                            continue;
                        }
                    }
                    serde_json::Value::Array(pair) if pair.len() == 2 => {
                        if let (Some(s), Some(e)) = (
                            json_value_to_duration(&pair[0]),
                            json_value_to_duration(&pair[1]),
                        ) {
                            if e > s {
                                out.push((s, e));
                            }
                        }
                        continue;
                    }
                    serde_json::Value::String(s) => {
                        if let Some(seg) = parse_range_line(s) {
                            out.push(seg);
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            if !out.is_empty() {
                return normalize_segments(out);
            }
            // JSON parsed but yielded nothing (e.g. `[]` or `[1,3]` quadrant
            // style): fall through to plain-text parsing of the raw string.
        }
    }

    // 2) Plain-text / line-oriented parsing.
    for line in text.lines() {
        if let Some(seg) = parse_range_line(line) {
            out.push(seg);
        }
    }
    // Also try the whole blob as a single range (single-line output).
    if out.is_empty() {
        if let Some(seg) = parse_range_line(text) {
            out.push(seg);
        }
    }

    normalize_segments(out)
}

fn json_value_to_duration(v: &serde_json::Value) -> Option<Duration> {
    match v {
        serde_json::Value::Number(n) => n
            .as_f64()
            .filter(|f| *f >= 0.0)
            .map(Duration::from_secs_f64),
        serde_json::Value::String(s) => parse_timestamp_token(s),
        _ => None,
    }
}

/// Parse one `"<start> <sep> <end>"` line. Separators: `-`, `–`, `—`,
/// `-->`, `→`, `to`, `,`, `;`.
fn parse_range_line(line: &str) -> Option<(Duration, Duration)> {
    let mut s = line.trim().to_string();
    if s.is_empty() {
        return None;
    }
    // Strip bullets / numbering / quotes / brackets.
    while let Some(c) = s.chars().next() {
        if c == '-' && s.len() > 1 && s.chars().nth(1).map(|n| n.is_ascii_digit()).unwrap_or(false) {
            break;
        }
        if "-*•[]\"'(){} ".contains(c) || c.is_ascii_digit() {
            if c.is_ascii_digit() {
                break;
            }
            s.remove(0);
            s = s.trim_start().to_string();
        } else {
            break;
        }
    }
    // Normalise multi-char separators to ASCII '-'.
    let normalised = s
        .replace("-->", "-")
        .replace("->", "-")
        .replace('→', "-")
        .replace('–', "-")
        .replace('—', "-");
    // Split on " to " (word separator) first, then on '-' / ',' / ';'.
    let candidates: Vec<String> = if normalised.to_lowercase().contains(" to ") {
        let lower = normalised.to_lowercase();
        let idx = lower.find(" to ").unwrap();
        vec![
            normalised[..idx].to_string(),
            normalised[idx + 4..].to_string(),
        ]
    } else if let Some(idx) = find_range_dash(&normalised) {
        vec![
            normalised[..idx].to_string(),
            normalised[idx + 1..].to_string(),
        ]
    } else if normalised.contains(',') || normalised.contains(';') {
        let sep = if normalised.contains(',') { ',' } else { ';' };
        normalised.split(sep).map(|p| p.to_string()).collect()
    } else {
        return None;
    };
    if candidates.len() < 2 {
        return None;
    }
    // Take the first token that parses as a timestamp on each side so
    // surrounding prose ("skip from X until Y") doesn't break parsing.
    let start = first_timestamp_in(&candidates[0])?;
    let end = first_timestamp_in(&candidates[candidates.len() - 1])?;
    if end > start {
        Some((start, end))
    } else {
        None
    }
}

/// Find the index of the `-` separating start/end, skipping the colons in
/// `MM:SS` / `HH:MM:SS`. Returns `None` when there is no such dash.
fn find_range_dash(s: &str) -> Option<usize> {
    // Prefer a dash surrounded by whitespace ("15 - 25").
    let bytes = s.as_bytes();
    for (i, w) in s.char_indices() {
        if w == '-' {
            let before_space = i > 0 && bytes[i - 1] == b' ';
            let after_space = bytes.get(i + 1) == Some(&b' ');
            if before_space || after_space {
                return Some(i);
            }
        }
    }
    // Otherwise the last dash followed by a digit (handles "00:15-00:25").
    let mut result = None;
    for (i, c) in s.char_indices() {
        if c == '-' && s[i + 1..].chars().next().map(|n| n.is_ascii_digit()).unwrap_or(false) {
            // Skip a leading negative-sign-like dash at position 0.
            if i == 0 {
                continue;
            }
            result = Some(i);
        }
    }
    result
}

/// Extract the first timestamp-like token from free-form text.
fn first_timestamp_in(fragment: &str) -> Option<Duration> {
    // Try the whole fragment first (common fast path).
    if let Some(d) = parse_timestamp_token(fragment) {
        return Some(d);
    }
    // Otherwise scan whitespace/comma-separated tokens.
    for token in fragment
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '[' || c == ']' || c == '"' || c == '\'' || c == '(' || c == ')')
    {
        if let Some(d) = parse_timestamp_token(token) {
            return Some(d);
        }
    }
    None
}

fn normalize_segments(mut segs: Vec<(Duration, Duration)>) -> Vec<(Duration, Duration)> {
    segs.retain(|(s, e)| *e > *s);
    segs.sort();
    segs.dedup();
    // Merge overlapping / adjacent segments.
    let mut merged: Vec<(Duration, Duration)> = Vec::with_capacity(segs.len());
    for (s, e) in segs {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                if e > last.1 {
                    last.1 = e;
                }
                continue;
            }
        }
        merged.push((s, e));
    }
    merged
}

/// Group extracted frames into small fixed-size batches for API-friendly grids.
///
/// `batch_size` is clamped to [`AI_SCAN_MAX_FRAMES_PER_BATCH`] so no batch
/// can ever grow into a 503-sized payload. E.g. 10 frames at batch size 4 →
/// `[4, 4, 2]`. The last batch may be smaller and still forms a valid
/// (partial) grid.
pub fn chunk_frames_into_batches(
    frames: &[(Duration, DynamicImage)],
    batch_size: usize,
) -> Vec<Vec<(Duration, DynamicImage)>> {
    if batch_size == 0 {
        return Vec::new();
    }
    let batch_size = batch_size.min(AI_SCAN_MAX_FRAMES_PER_BATCH);
    frames
        .chunks(batch_size)
        .map(|c| c.to_vec())
        .collect()
}

/// Extract frames from `video_path` at one frame every `frame_step_secs`
/// seconds (step 1 = 1fps), downscaled to ≤320px wide.
///
/// `frame_step_secs` comes from [`adaptive_frame_step_secs`]: long videos use
/// 2–3s steps so a 20-minute video yields ~450 frames instead of 1361.
///
/// Robust for long videos:
/// 1. Uses a dedicated ABSOLUTE temp dir (`temp_dir()/otip_scan`).
/// 2. Calls `create_dir_all` BEFORE spawning FFmpeg so the muxer can always
///    open its outputs (fixes `Could not open file: frame_*.jpg` midway).
/// 3. Passes an ABSOLUTE output pattern to FFmpeg — never relies on the
///    process working directory.
/// 4. Blocks on `wait()` until FFmpeg is 100% finished before reading frames.
/// 5. Returns the temp dir to the caller WITHOUT deleting it; the caller
///    (`run_ai_scan`) deletes it only after ALL grid/AI work is finished.
///
/// Returns `(frames, temp_dir)`. Empty frames (ffmpeg missing/unreadable)
/// means the caller should fall back to the text probe.
///
/// `progress_tx` receives live extraction updates (≈0.0→`AI_SCAN_EXTRACTION_END`);
/// `expected_frames` (from ffprobe ÷ step) scales them.
/// Without an expectation a small pulsed fraction is emitted instead.
async fn extract_frames_1fps(
    video_path: &std::path::Path,
    progress_tx: &Option<tokio::sync::mpsc::UnboundedSender<AiScanProgressUpdate>>,
    expected_frames: Option<u64>,
    frame_step_secs: u64,
) -> (Vec<(Duration, DynamicImage)>, std::path::PathBuf) {
    use std::time::Instant;

    let started = Instant::now();
    // 1. Absolute dedicated temp directory.
    let temp_dir = std::env::temp_dir().join("otip_scan");
    // 2. Ensure it exists BEFORE spawning FFmpeg.
    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        warn!("extract_frames_1fps: cannot create temp dir {}: {e}", temp_dir.display());
        return (Vec::new(), temp_dir);
    }
    // Drop stale frames from previous runs so a new scan never mixes old and
    // new outputs (stale `frame_*.jpg` would corrupt timestamps/counts).
    // Also sweeps the pre-rename legacy dir so upgrades never pick up orphans.
    for dir in [&temp_dir, &std::env::temp_dir().join("otip_scan_frames")] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.filter_map(|e| e.ok()) {
                let p = e.path();
                let is_frame_jpg = p.extension().and_then(|x| x.to_str()) == Some("jpg")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("frame_"))
                        .unwrap_or(false);
                if is_frame_jpg {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }

    // 3. Absolute output pattern — do NOT rely on the working directory.
    let output_pattern = temp_dir.join("frame_%05d.jpg");
    let output_str = output_pattern.to_string_lossy().to_string();
    let frame_step_secs = frame_step_secs.max(1);
    let fps_filter = format!("fps=1/{frame_step_secs},scale=320:-1");

    // Adaptive rate, max width 320px (matches MAX_FRAME_WIDTH), JPEG q5.
    // 4. Spawn FFmpeg, then BLOCK on wait() until it is 100% finished
    // generating all frames. While waiting, poll the output dir so the UI
    // progress bar keeps moving on long extractions (no frozen-app feel).
    let mut child = match tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            &video_path.to_string_lossy(),
            "-vf",
            &fps_filter,
            "-q:v",
            "5",
            "-y",
            &output_str,
        ])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            warn!("extract_frames_1fps: cannot spawn ffmpeg: {e}");
            return (Vec::new(), temp_dir);
        }
    };
    // Concurrent progress polling: tick every 250ms, count finished frames.
    // `child.wait()` is re-polled each lap; nothing below the loop runs until
    // FFmpeg has fully exited. (First tick fires immediately → instant UI feedback.)
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            _ = ticker.tick() => {
                let done = count_frame_jpgs(&temp_dir);
                let frac = extraction_progress(done, expected_frames);
                match expected_frames {
                    Some(exp) => emit_progress(
                        progress_tx,
                        frac,
                        format!("Extracting frames {done}/{exp}..."),
                    ),
                    None => emit_progress(
                        progress_tx,
                        frac,
                        format!("Extracting frames {done}..."),
                    ),
                }
            }
        }
    };
    let status = match status {
        Ok(status) => status,
        Err(e) => {
            warn!("extract_frames_1fps: ffmpeg wait failed: {e}");
            return (Vec::new(), temp_dir);
        }
    };
    if !status.success() {
        warn!("extract_frames_1fps: ffmpeg failed for {}", video_path.display());
        // NOTE: no cleanup here — the caller deletes temp_dir after its
        // fallback probe finishes (safe-cleanup contract).
        return (Vec::new(), temp_dir);
    }

    // Read back sorted JPEGs ONLY after wait() confirmed FFmpeg is done:
    // frame i ↔ timestamp i × step seconds (step 1 = 1fps).
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&temp_dir) {
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("jpg") {
                paths.push(p);
            }
        }
    }
    paths.sort();

    let mut out = Vec::with_capacity(paths.len());
    for (idx, p) in paths.iter().enumerate() {
        match image::open(p) {
            Ok(img) => out.push((
                Duration::from_secs(idx as u64 * frame_step_secs),
                img,
            )),
            Err(e) => warn!("extract_frames_1fps: cannot decode {}: {e}", p.display()),
        }
    }
    // NOTE: no cleanup here — temp_dir is returned and deleted by run_ai_scan
    // only AFTER all grid processing and AI scanning are completely finished.
    info!(
        "extract_frames_1fps: {} frame(s) every {}s from {} in {:?} (dir {})",
        out.len(),
        frame_step_secs,
        video_path.display(),
        started.elapsed(),
        temp_dir.display(),
    );
    emit_progress(
        progress_tx,
        AI_SCAN_EXTRACTION_END,
        format!("Extracted {} frames...", out.len()),
    );
    (out, temp_dir)
}

/// Send ONE small grid batch to Gemini and parse its skip segments.
///
/// Rate-limit aware: 429/503 responses are retried with exponential backoff
/// (10s, 20s, 40s, up to [`AI_SCAN_MAX_RETRIES`]) before giving up on the
/// batch. Any other failure yields an empty vec immediately.
///
/// Never panics / never returns Err to the scan loop: failures only skip
/// their batch so sequential processing continues with the next one.
#[allow(clippy::too_many_arguments)]
async fn send_grid_batch(
    client: &reqwest::Client,
    url: &str,
    file_name: &str,
    effective_prompt: &str,
    batch_idx: usize,
    batch_count: usize,
    batch: &[(Duration, DynamicImage)],
    grid_jpeg: Vec<u8>,
    progress_tx: &Option<tokio::sync::mpsc::UnboundedSender<AiScanProgressUpdate>>,
) -> Vec<(Duration, Duration)> {
    use base64::Engine as _;
    let base64_image = base64::engine::general_purpose::STANDARD.encode(&grid_jpeg);
    let stamps: Vec<u64> = batch.iter().map(|(t, _)| t.as_secs()).collect();
    let batch_start = batch.first().map(|(t, _)| t.as_secs()).unwrap_or(0);
    let instruction = format!(
        "Video file under analysis: \"{file_name}\" (batch {}/{}, frames at seconds {:?}, sampled through the video).\n\
         User skip request: {effective_prompt}\n\
         Analyse these {} frame(s) in time order. \
         Return ONLY a JSON array of objects with numeric \"start\" and \"end\" \
         fields in ABSOLUTE video seconds, e.g. [{{\"start\": {batch_start}, \"end\": {}}}] \
         covering the parts matching the skip request. \
         If nothing should be skipped, return [].",
        batch_idx + 1,
        batch_count,
        stamps,
        batch.len(),
        batch_start + 1,
    );
    let payload = serde_json::json!({
        "contents": [{
            "parts": [
                { "text": instruction },
                { "inline_data": { "mime_type": "image/jpeg", "data": base64_image } }
            ]
        }],
        "generationConfig": {
            "temperature": 0.1,
            "maxOutputTokens": 512,
            "responseMimeType": "application/json"
        }
    });
    // Retry loop for rate limiting: re-POST while Gemini says 429/503.
    let mut attempt: u32 = 0;
    let text: String = loop {
        let response = match client.post(url).json(&payload).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("send_grid_batch {}/{}: request failed: {e}", batch_idx + 1, batch_count);
                return Vec::new();
            }
        };
        let status = response.status();
        if is_retryable_status(status.as_u16()) {
            if attempt >= AI_SCAN_MAX_RETRIES {
                warn!(
                    "send_grid_batch {}/{}: {} persisted after {} retries, skipping batch",
                    batch_idx + 1,
                    batch_count,
                    status,
                    AI_SCAN_MAX_RETRIES,
                );
                return Vec::new();
            }
            attempt += 1;
            let backoff = retry_backoff_secs(attempt);
            warn!(
                "send_grid_batch {}/{}: {} — retry {}/{} after {}s",
                batch_idx + 1,
                batch_count,
                status,
                attempt,
                AI_SCAN_MAX_RETRIES,
                backoff,
            );
            emit_progress(
                progress_tx,
                batch_progress(batch_idx, batch_count),
                format!(
                    "Rate limited, retrying batch {}/{} ({status})...",
                    batch_idx + 1,
                    batch_count,
                ),
            );
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            continue;
        }
        if !status.is_success() {
            warn!("send_grid_batch {}/{}: Gemini status {}", batch_idx + 1, batch_count, status);
            return Vec::new();
        }
        let json: serde_json::Value = match response.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!("send_grid_batch {}/{}: response JSON failed: {e}", batch_idx + 1, batch_count);
                return Vec::new();
            }
        };
        break json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
    };
    if text.trim().is_empty() {
        debug!("send_grid_batch {}/{}: empty model reply", batch_idx + 1, batch_count);
        return Vec::new();
    }
    parse_ai_timestamps(&text)
}

/// Text-only fallback probe (no frames): asks Gemini for skip ranges from the
/// file name + custom prompt. Used when 1fps extraction yields nothing
/// (ffmpeg missing / unsupported file) so the bridge stays functional.
async fn run_text_probe(
    client: &reqwest::Client,
    url: &str,
    file_name: &str,
    effective_prompt: &str,
) -> Vec<(Duration, Duration)> {
    let instruction = format!(
        "Video file under analysis: \"{file_name}\".\n\
         User skip request: {effective_prompt}\n\
         The full video will be analysed frame-by-frame after this probe. \
         For now, based on the request, reply with the skip segments as data.\n\
         Return ONLY a JSON array of objects with numeric \"start\" and \"end\" \
         fields in seconds, e.g. [{{\"start\": 15, \"end\": 25}}]. \
         If nothing should be skipped, return []."
    );
    let payload = serde_json::json!({
        "contents": [{ "parts": [{ "text": instruction }] }],
        "generationConfig": {
            "temperature": 0.1,
            "maxOutputTokens": 512,
            "responseMimeType": "application/json"
        }
    });
    let response = match client.post(url).json(&payload).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("run_ai_scan text probe: request failed: {e}");
            return Vec::new();
        }
    };
    if !response.status().is_success() {
        warn!("run_ai_scan text probe: Gemini status {}", response.status());
        return Vec::new();
    }
    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            warn!("run_ai_scan text probe: response JSON failed: {e}");
            return Vec::new();
        }
    };
    let text = json
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.get(0))
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    if text.trim().is_empty() {
        debug!("run_ai_scan text probe: empty model reply");
        return Vec::new();
    }
    parse_ai_timestamps(text)
}

/// Execution bridge for the `otip-app` "Start AI Scan" button.
///
/// Wires the UI inputs (`video_path`, `gemini_api_key`, `gemini_model`,
/// `ai_skip_prompt`) to the existing [`VideoScanner`]/Gemini logic in this
/// module and returns parsed skip segments.
///
/// Strict chunking pipeline (avoids one massive 503-prone payload):
/// 1. Extraction — frames sampled via ffmpeg at an adaptive rate
///    ([`adaptive_frame_step_secs`]: 1fps ≤5min, 1/2s ≤15min, 1/3s beyond),
///    with live progress (0%→30%).
/// 2. Combining — frames grouped into chunks of at most
///    [`AI_SCAN_MAX_FRAMES_PER_BATCH`] (default [`AI_SCAN_FRAMES_PER_BATCH`]);
///    one small grid image is generated PER batch, never one massive grid
///    (30%→40%, `"Combining images into batches..."`).
/// 3. Sequential — batch grids sent one-by-one to Gemini
///    (`"Sending batch X of Y to AI..."`, 40%→90%), paced 4s apart with
///    exponential-backoff retries on 429/503 (up to [`AI_SCAN_MAX_RETRIES`]).
/// 4. Merge + cleanup — [`normalize_segments`], temp dir deleted, 100%.
///
/// `progress_tx` (when `Some`) receives `(fraction 0.0..=1.0, label)` updates
/// throughout extraction and batching so the UI progress bar stays alive.
/// Send failures (UI gone) are ignored; the scan always runs to completion.
///
/// Behaviour:
/// - Empty API key → returns empty vec (caller shows a "missing key" status).
/// - Missing file → returns empty vec with a warning log.
/// - Extraction yields nothing (no ffmpeg / unreadable file) → falls back to
///   the text probe so the bridge stays functional.
/// - Per-batch network/parse failures are logged and skipped; remaining
///   batches still run. Never panics — safe as an Iced `Task::perform` future.
pub async fn run_ai_scan(
    video_path: PathBuf,
    api_key: String,
    model: String,
    prompt: String,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<AiScanProgressUpdate>>,
) -> Vec<(Duration, Duration)> {
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        warn!("run_ai_scan: refusing without a Gemini API key");
        return Vec::new();
    }
    if !video_path.exists() {
        warn!("run_ai_scan: video path does not exist: {}", video_path.display());
        return Vec::new();
    }

    // Reuse the existing scanner configuration so model/endpoint stay in
    // sync with the rest of the app (GridScanner / AI logic).
    // (frame_interval is set to the adaptive step once probed, below.)
    let mut scanner_config = ScannerConfig::default();
    scanner_config.api_key = api_key.clone();
    let model = model.trim();
    scanner_config.model = if model.is_empty() {
        crate::config::GEMINI_DEFAULT_MODEL.to_string()
    } else {
        model.to_string()
    };

    let user_prompt = prompt.trim();
    let effective_prompt = if user_prompt.is_empty() {
        DEFAULT_AI_SKIP_PROMPT.to_string()
    } else {
        user_prompt.to_string()
    };

    let file_name = video_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("video")
        .to_string();

    info!(
        "run_ai_scan: scanning {} with model {} (prompt {} chars)",
        video_path.display(),
        scanner_config.model,
        effective_prompt.len()
    );

    let url = format!(
        "{}/{}:generateContent?key={}",
        scanner_config.endpoint, scanner_config.model, scanner_config.api_key
    );

    let client = reqwest::Client::builder()
        // Graceful handling of API congestion: fail after 60s instead of hanging.
        .timeout(std::time::Duration::from_secs(GEMINI_TIMEOUT_SECS))
        .build();
    let client = match client {
        Ok(c) => c,
        Err(e) => {
            warn!("run_ai_scan: HTTP client build failed: {e}");
            return Vec::new();
        }
    };

    // 1. Extraction at an adaptive rate. `temp_dir` is kept alive for the
    // whole scan — it is deleted only after ALL grid/AI work finishes below.
    // Probe duration first: it scales extraction progress AND picks the frame
    // step (1fps ⇒ expected frames ≈ duration seconds).
    emit_progress(&progress_tx, 0.0, "Starting AI scan...");
    let duration_secs = probe_duration_secs(&video_path).await;
    let frame_step_secs = adaptive_frame_step_secs(duration_secs);
    if frame_step_secs > 1 {
        info!(
            "run_ai_scan: {}s video → 1 frame every {}s (adaptive extraction)",
            duration_secs.unwrap_or(0),
            frame_step_secs,
        );
    }
    let expected_frames = duration_secs.map(|d| d / frame_step_secs.max(1));
    let (frames, temp_dir) =
        extract_frames_1fps(&video_path, &progress_tx, expected_frames, frame_step_secs).await;
    if frames.is_empty() {
        debug!("run_ai_scan: no frames extracted, falling back to text probe");
        emit_progress(&progress_tx, 0.4, "Contacting AI...");
        let segs = run_text_probe(&client, &url, &file_name, &effective_prompt).await;
        // Safe cleanup: temp dir goes away only after the fallback probe is done.
        let _ = std::fs::remove_dir_all(&temp_dir);
        emit_progress(&progress_tx, 1.0, "AI scan complete");
        info!("run_ai_scan text probe: parsed {} skip segment(s)", segs.len());
        return segs;
    }

    // 2. Combining: small fixed groups (default 4 frames per grid image,
    // hard-capped at AI_SCAN_MAX_FRAMES_PER_BATCH). NEVER one massive grid.
    emit_progress(&progress_tx, AI_SCAN_EXTRACTION_END, "Combining images into batches...");
    let batches = chunk_frames_into_batches(&frames, AI_SCAN_FRAMES_PER_BATCH);
    info!(
        "run_ai_scan: {} frame(s) → {} batch(es) of ≤{}",
        frames.len(),
        batches.len(),
        AI_SCAN_FRAMES_PER_BATCH
    );

    // Reuse the downscale + JPEG grid builder so batches match the live
    // `VideoScanner` payload (≤320px cells, JPEG q75).
    // NOTE: `scan_progress_tx` is the VideoScanner channel — distinct from the
    // `progress_tx` streaming-AI-scan sender above (no shadowing).
    scanner_config.frame_interval = Duration::from_secs(frame_step_secs);
    let (scan_progress_tx, _) = mpsc::unbounded_channel();
    let (segment_tx, _) = mpsc::unbounded_channel();
    let scanner = VideoScanner::new(scanner_config, scan_progress_tx, segment_tx);
    let batch_count = batches.len();

    // Phase 2: generate ONE separate grid image per batch (30%→40%).
    let mut grids: Vec<(Vec<(Duration, DynamicImage)>, Vec<u8>)> =
        Vec::with_capacity(batch_count);
    for (idx, batch) in batches.into_iter().enumerate() {
        match scanner.create_grid_image(&batch) {
            Ok(jpeg) => grids.push((batch, jpeg)),
            Err(e) => {
                warn!("run_ai_scan batch {}/{}: grid build failed: {e}", idx + 1, batch_count);
                continue;
            }
        }
        emit_progress(
            &progress_tx,
            combining_progress(grids.len(), batch_count),
            "Combining images into batches...".to_string(),
        );
    }
    let grid_count = grids.len();
    if grid_count == 0 {
        warn!("run_ai_scan: no batch grids built");
        let _ = std::fs::remove_dir_all(&temp_dir);
        emit_progress(&progress_tx, 1.0, "AI scan complete");
        return Vec::new();
    }

    // 3. Sequential processing: send the batch grids one-by-one, in order —
    // each batch emits progress so the bar moves through long batch runs.
    let mut all_segments: Vec<(Duration, Duration)> = Vec::new();
    for (idx, (batch, grid_jpeg)) in grids.iter().enumerate() {
        emit_progress(
            &progress_tx,
            batch_progress(idx, grid_count),
            format!("Sending batch {}/{} to AI...", idx + 1, grid_count),
        );
        let segs = send_grid_batch(
            &client,
            &url,
            &file_name,
            &effective_prompt,
            idx,
            grid_count,
            batch,
            grid_jpeg.clone(),
            &progress_tx,
        )
        .await;
        all_segments.extend(segs);
        // Rate limiting: 4s pacing at the end of each iteration so hundreds
        // of back-to-back batches don't trip Google's free-tier quotas.
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    }

    // 4. Merge results from all batches into the final segment list.
    emit_progress(&progress_tx, AI_SCAN_ANALYSIS_END, "Merging results...");
    let merged = normalize_segments(all_segments);
    // 5. Safe cleanup: delete the temp dir ONLY after all grid processing
    // and AI scanning are completely finished.
    let _ = std::fs::remove_dir_all(&temp_dir);
    emit_progress(&progress_tx, 1.0, "AI scan complete");
    info!("run_ai_scan: merged {} skip segment(s) from {grid_count} batch(es)", merged.len());
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn test_grid_creation() {
        let config = ScannerConfig::default();
        let (progress_tx, _) = mpsc::unbounded_channel();
        let (segment_tx, _) = mpsc::unbounded_channel();
        
        let scanner = VideoScanner::new(config, progress_tx, segment_tx);
        
        // Create 4 test frames
        let frames: Vec<_> = (0..4).map(|i| {
            let mut img = ImageBuffer::new(320, 240);
            for (x, y, pixel) in img.enumerate_pixels_mut() {
                *pixel = Rgb([(i * 50) as u8, (i * 30) as u8, (i * 10) as u8]);
            }
            (Duration::from_secs(i), DynamicImage::ImageRgb8(img))
        }).collect();

        let result = scanner.create_grid_image(&frames);
        assert!(result.is_ok());
        let grid_bytes = result.unwrap();
        assert!(!grid_bytes.is_empty());
        
        // Verify it's a valid PNG
        let grid_img = image::load_from_memory(&grid_bytes).unwrap();
        assert_eq!(grid_img.width(), 640); // 2 * 320
        assert_eq!(grid_img.height(), 480); // 2 * 240
    }

    #[test]
    fn test_parse_timestamp_token() {
        assert_eq!(parse_timestamp_token("75"), Some(Duration::from_secs(75)));
        assert_eq!(parse_timestamp_token("75s"), Some(Duration::from_secs(75)));
        assert_eq!(
            parse_timestamp_token("01:15"),
            Some(Duration::from_secs(75))
        );
        assert_eq!(
            parse_timestamp_token("01:02:03"),
            Some(Duration::from_secs(3723))
        );
        assert!(parse_timestamp_token("").is_none());
    }

    #[test]
    fn test_parse_ai_timestamps_json_objects() {
        let segs = parse_ai_timestamps(r#"[{"start": 15, "end": 25}, {"start": "00:30", "end": "00:40"}]"#);
        assert_eq!(
            segs,
            vec![
                (Duration::from_secs(15), Duration::from_secs(25)),
                (Duration::from_secs(30), Duration::from_secs(40)),
            ]
        );
    }

    #[test]
    fn test_parse_ai_timestamps_plain_text() {
        let segs = parse_ai_timestamps("00:15-00:25\n15s to 25s\n");
        assert!(!segs.is_empty());
        assert_eq!(segs[0], (Duration::from_secs(15), Duration::from_secs(25)));
    }

    #[test]
    fn test_parse_ai_timestamps_empty() {
        assert!(parse_ai_timestamps("[]").is_empty());
        assert!(parse_ai_timestamps("no segments here").is_empty());
    }

    fn dummy_frames(n: u64) -> Vec<(Duration, DynamicImage)> {
        (0..n)
            .map(|i| {
                let img = ImageBuffer::from_pixel(8, 8, Rgb([i as u8, 0, 0]));
                (Duration::from_secs(i), DynamicImage::ImageRgb8(img))
            })
            .collect()
    }

    #[test]
    fn test_chunk_frames_into_batches_of_four() {
        // 10 frames at 1fps → batches [4, 4, 2]: small API-friendly grids.
        let frames = dummy_frames(10);
        let batches = chunk_frames_into_batches(&frames, AI_SCAN_FRAMES_PER_BATCH);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 4);
        assert_eq!(batches[1].len(), 4);
        assert_eq!(batches[2].len(), 2);
        // Timestamps preserved for absolute segment mapping.
        assert_eq!(batches[0][0].0, Duration::from_secs(0));
        assert_eq!(batches[1][0].0, Duration::from_secs(4));
        assert_eq!(batches[2][0].0, Duration::from_secs(8));
        assert_eq!(AI_SCAN_FRAME_INTERVAL, Duration::from_secs(1));
    }

    #[test]
    fn test_chunk_frames_empty_and_zero_size() {
        assert!(chunk_frames_into_batches(&[], 4).is_empty());
        assert!(chunk_frames_into_batches(&dummy_frames(4), 0).is_empty());
    }

    #[test]
    fn test_chunk_frames_respects_max_batch_cap() {
        // Even a huge request never exceeds AI_SCAN_MAX_FRAMES_PER_BATCH.
        let batches = chunk_frames_into_batches(&dummy_frames(100), 10_000);
        assert!(batches.iter().all(|b| b.len() <= AI_SCAN_MAX_FRAMES_PER_BATCH));
        assert_eq!(
            batches.len(),
            100_usize.div_ceil(AI_SCAN_MAX_FRAMES_PER_BATCH)
        );
    }

    #[test]
    fn test_merge_batched_segments() {
        // Simulates merging per-batch Gemini replies into the final vec.
        let mut all = Vec::new();
        all.extend(parse_ai_timestamps(r#"[{"start": 0, "end": 2}]"#));
        all.extend(parse_ai_timestamps(r#"[{"start": 5, "end": 7}]"#));
        all.extend(parse_ai_timestamps(r#"[]"#));
        let merged = normalize_segments(all);
        assert_eq!(
            merged,
            vec![
                (Duration::from_secs(0), Duration::from_secs(2)),
                (Duration::from_secs(5), Duration::from_secs(7)),
            ]
        );
    }

    #[tokio::test]
    async fn test_run_ai_scan_rejects_missing_key() {
        let segs = run_ai_scan(
            PathBuf::from("/tmp/otip-test-video.mp4"),
            String::new(),
            "gemini-3.8-flash".to_string(),
            "skip ads".to_string(),
            None,
        )
        .await;
        assert!(segs.is_empty());
    }

    #[tokio::test]
    async fn test_run_ai_scan_missing_key_sends_no_progress() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let segs = run_ai_scan(
            PathBuf::from("/tmp/otip-test-video.mp4"),
            String::new(),
            "gemini-3.8-flash".to_string(),
            "skip ads".to_string(),
            Some(tx),
        )
        .await;
        assert!(segs.is_empty());
        // Early validation return: no progress updates emitted, no deadlock.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_batch_progress_mapping() {
        // Phase 3 spans COMBINING_END→ANALYSIS_END across batches.
        assert!((batch_progress(0, 4) - AI_SCAN_COMBINING_END).abs() < f32::EPSILON);
        let mid = batch_progress(2, 4);
        assert!(mid > AI_SCAN_COMBINING_END && mid < AI_SCAN_ANALYSIS_END);
        assert!(batch_progress(1, 4) < batch_progress(3, 4));
        assert!(batch_progress(3, 4) < AI_SCAN_ANALYSIS_END);
        assert_eq!(batch_progress(0, 0), AI_SCAN_ANALYSIS_END);
    }

    #[test]
    fn test_extraction_progress_mapping() {
        assert_eq!(extraction_progress(0, Some(10)), 0.0);
        assert!((extraction_progress(5, Some(10)) - AI_SCAN_EXTRACTION_END * 0.5).abs() < 1e-6);
        assert!((extraction_progress(10, Some(10)) - AI_SCAN_EXTRACTION_END).abs() < 1e-6);
        // Over-count clamps at the phase end; unknown length pulses nonzero.
        assert!((extraction_progress(99, Some(10)) - AI_SCAN_EXTRACTION_END).abs() < 1e-6);
        assert_eq!(extraction_progress(3, None), 0.1);
    }

    #[test]
    fn test_combining_progress_mapping() {
        assert!((combining_progress(0, 4) - AI_SCAN_EXTRACTION_END).abs() < f32::EPSILON);
        assert!((combining_progress(4, 4) - AI_SCAN_COMBINING_END).abs() < 1e-6);
        assert!(combining_progress(1, 4) < combining_progress(3, 4));
        assert_eq!(combining_progress(0, 0), AI_SCAN_COMBINING_END);
    }

    #[test]
    fn test_emit_progress_clamps_and_drops_safely() {
        // None receiver: no-op, never panics.
        emit_progress(&None, 0.5, "hello");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        emit_progress(&Some(tx), 1.5, "over");
        let (p, label) = rx.try_recv().expect("update must arrive");
        assert_eq!(p, 1.0);
        assert_eq!(label, "over");
    }

    #[test]
    fn test_adaptive_frame_step_secs() {
        // Short videos keep full 1fps; >5min steps out to protect the API.
        assert_eq!(adaptive_frame_step_secs(None), 1);
        assert_eq!(adaptive_frame_step_secs(Some(0)), 1);
        assert_eq!(adaptive_frame_step_secs(Some(60)), 1);
        assert_eq!(adaptive_frame_step_secs(Some(300)), 1);
        assert_eq!(adaptive_frame_step_secs(Some(301)), 2);
        assert_eq!(adaptive_frame_step_secs(Some(900)), 2);
        assert_eq!(adaptive_frame_step_secs(Some(901)), 3);
        assert_eq!(adaptive_frame_step_secs(Some(1361)), 3);
        // The reported blowup: 1361 frames → ~454 frames → ~114 batches.
        let frames_22min = 1361u64 / adaptive_frame_step_secs(Some(1361));
        assert!(frames_22min < 1361 / 2);
    }

    #[test]
    fn test_is_retryable_status() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(503));
        for code in [200, 201, 400, 401, 403, 404, 500, 502, 504] {
            assert!(!is_retryable_status(code), "{code} must not retry");
        }
    }

    #[test]
    fn test_retry_backoff_secs_is_exponential() {
        assert_eq!(AI_SCAN_MAX_RETRIES, 3);
        assert_eq!(retry_backoff_secs(1), 10);
        assert_eq!(retry_backoff_secs(2), 20);
        assert_eq!(retry_backoff_secs(3), 40);
        assert!(retry_backoff_secs(2) > retry_backoff_secs(1));
    }

    #[test]
    fn test_rate_limit_constants() {
        assert_eq!(AI_SCAN_BATCH_DELAY_SECS, 4);
        assert_eq!(AI_SCAN_RETRY_BASE_DELAY_SECS, 10);
        assert_eq!(AI_SCAN_LONG_VIDEO_SECS, 300);
    }
}
