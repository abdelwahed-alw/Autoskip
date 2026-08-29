//! Video scanning logic and grid processing

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
}