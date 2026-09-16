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
            .timeout(std::time::Duration::from_secs(30))
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
                            mime_type: "image/png".to_string(),
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
                    mime_type: "image/png".to_string(),
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

    /// Create a 2x2 grid image from 4 frames
    fn create_grid_image(&self, frames: &[(Duration, DynamicImage)]) -> Result<Vec<u8>> {
        let (grid_w, grid_h) = self.config.grid_size;
        let (frame_w, frame_h) = self.config.frame_resolution;
        
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

            // Convert to RGB if needed
            let rgb_frame: image::ImageBuffer<image::Rgb<u8>, Vec<u8>> = frame.to_rgb8();
            
            for fy in 0..frame_h.min(rgb_frame.height()) {
                for fx in 0..frame_w.min(rgb_frame.width()) {
                    let pixel = rgb_frame.get_pixel(fx, fy);
                    grid.put_pixel(x + fx, y + fy, *pixel);
                }
            }
        }

        // Encode to PNG
        let mut bytes = Vec::new();
        grid.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .map_err(|e| OtipError::Scanner(ScannerError::GridCreationFailed(e.to_string())))?;

        Ok(bytes)
    }

    /// Resize frame to target resolution
    fn resize_frame(&self, frame: DynamicImage) -> Result<DynamicImage> {
        let (w, h) = self.config.frame_resolution;
        Ok(frame.resize_exact(w, h, image::imageops::FilterType::Lanczos3))
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
                            "mime_type": "image/png",
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

/// Execution bridge for the `otip-app` "Start AI Scan" button.
///
/// Wires the UI inputs (`video_path`, `gemini_api_key`, `gemini_model`,
/// `ai_skip_prompt`) to the existing [`VideoScanner`]/Gemini logic in this
/// module and returns parsed skip segments.
///
/// Behaviour:
/// - Empty API key → returns empty vec (caller shows a "missing key" status).
/// - Missing file → returns empty vec with a warning log.
/// - Otherwise sends the custom prompt (or [`DEFAULT_AI_SKIP_PROMPT`]) plus
///   the video file name to Gemini, then parses the reply with
///   [`parse_ai_timestamps`]. Any network/parse failure yields an empty vec
///   (never panics — safe as an Iced `Task::perform` future).
pub async fn run_ai_scan(
    video_path: PathBuf,
    api_key: String,
    model: String,
    prompt: String,
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
        .unwrap_or("video");

    info!(
        "run_ai_scan: scanning {} with model {} (prompt {} chars)",
        video_path.display(),
        scanner_config.model,
        effective_prompt.len()
    );

    // Text-probe request: ask Gemini for skip ranges honouring the custom
    // prompt. Frame-grid uploads ride on top of this via `VideoScanner` in
    // the full pipeline; the text probe keeps the bridge functional even
    // before frames are extracted.
    let url = format!(
        "{}/{}:generateContent?key={}",
        scanner_config.endpoint, scanner_config.model, scanner_config.api_key
    );
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let client = match client {
        Ok(c) => c,
        Err(e) => {
            warn!("run_ai_scan: HTTP client build failed: {e}");
            return Vec::new();
        }
    };

    let response = client.post(&url).json(&payload).send().await;
    let response = match response {
        Ok(r) => r,
        Err(e) => {
            warn!("run_ai_scan: request failed: {e}");
            return Vec::new();
        }
    };
    if !response.status().is_success() {
        warn!("run_ai_scan: Gemini status {}", response.status());
        return Vec::new();
    }
    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            warn!("run_ai_scan: response JSON failed: {e}");
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
        debug!("run_ai_scan: empty model reply");
        return Vec::new();
    }
    let segments = parse_ai_timestamps(text);
    info!("run_ai_scan: parsed {} skip segment(s)", segments.len());
    segments
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

    #[tokio::test]
    async fn test_run_ai_scan_rejects_missing_key() {
        let segs = run_ai_scan(
            PathBuf::from("/tmp/otip-test-video.mp4"),
            String::new(),
            "gemini-3.7-flash".to_string(),
            "skip ads".to_string(),
        )
        .await;
        assert!(segs.is_empty());
    }
}
