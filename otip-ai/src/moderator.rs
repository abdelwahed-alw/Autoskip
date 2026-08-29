//! Content moderator orchestrator

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, debug, warn, error};
use otip_core::domain::{VideoId, ScanSegment, QuadrantFlags, TimelineSegment, TimelineSegmentType, ScanProgress};
use otip_core::error::{Result, OtipError};
use otip_core::scan::ScannerConfig;
use crate::gemini::{GeminiClient, GeminiConfig};
use crate::grid_processor::{GridProcessor, GridConfig};

/// Configuration for the content moderator
#[derive(Debug, Clone)]
pub struct ModerationConfig {
    pub scanner_config: ScannerConfig,
    pub gemini_config: GeminiConfig,
    pub grid_config: GridConfig,
    pub auto_skip_enabled: bool,
    pub skip_buffer_ms: u64,
    pub confidence_threshold: f32,
}

impl Default for ModerationConfig {
    fn default() -> Self {
        Self {
            scanner_config: ScannerConfig::default(),
            gemini_config: GeminiConfig::default(),
            grid_config: GridConfig::default(),
            auto_skip_enabled: true,
            skip_buffer_ms: 500,
            confidence_threshold: 0.7,
        }
    }
}

/// Statistics for the moderator
#[derive(Debug, Default, Clone)]
pub struct ModeratorStats {
    pub videos_processed: u64,
    pub total_segments_scanned: u64,
    pub explicit_segments_found: u64,
    pub segments_skipped: u64,
    pub api_calls: u64,
    pub api_errors: u64,
}

/// Main content moderator
pub struct ContentModerator {
    config: ModerationConfig,
    gemini_client: GeminiClient,
    grid_processor: GridProcessor,
    stats: Arc<RwLock<ModeratorStats>>,
    progress_tx: mpsc::UnboundedSender<ScanProgress>,
    segment_tx: mpsc::UnboundedSender<(VideoId, TimelineSegment)>,
    active_scans: Arc<RwLock<std::collections::HashMap<VideoId, ScanState>>>,
}

#[derive(Debug)]
struct ScanState {
    video_id: VideoId,
    total_duration: Duration,
    scanned_duration: Duration,
    segments_found: usize,
    explicit_segments: usize,
    is_cancelled: bool,
}

impl ContentModerator {
    pub fn new(
        config: ModerationConfig,
        progress_tx: mpsc::UnboundedSender<ScanProgress>,
        segment_tx: mpsc::UnboundedSender<(VideoId, TimelineSegment)>,
    ) -> Result<Self> {
        let gemini_client = GeminiClient::new(config.gemini_config.clone())?;
        let grid_processor = GridProcessor::new(config.grid_config.clone());

        Ok(Self {
            config,
            gemini_client,
            grid_processor,
            stats: Arc::new(RwLock::new(ModeratorStats::default())),
            progress_tx,
            segment_tx,
            active_scans: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Start moderating a video
    pub async fn moderate_video(
        &self,
        video_id: VideoId,
        video_path: String,
        total_duration: Duration,
        frame_receiver: mpsc::UnboundedReceiver<(Duration, image::DynamicImage)>,
    ) -> Result<()> {
        info!("Starting content moderation for video {}", video_id);

        // Initialize scan state
        {
            let mut scans = self.active_scans.write().await;
            scans.insert(video_id, ScanState {
                video_id,
                total_duration,
                scanned_duration: Duration::ZERO,
                segments_found: 0,
                explicit_segments: 0,
                is_cancelled: false,
            });
        }

        let mut scanned_duration = Duration::ZERO;
        let mut segments_found = 0;
        let mut explicit_segments = 0;
        let mut frame_buffer = Vec::new();
        let frames_per_grid = (self.config.scanner_config.grid_size.0 * self.config.scanner_config.grid_size.1) as usize;
        let mut grid_index = 0u32;

        let mut frame_receiver = frame_receiver;
        
        while let Some((timestamp, frame)) = frame_receiver.recv().await {
            // Check if scan was cancelled
            {
                let scans = self.active_scans.read().await;
                if let Some(state) = scans.get(&video_id) {
                    if state.is_cancelled {
                        info!("Scan cancelled for video {}", video_id);
                        break;
                    }
                }
            }

            // Resize frame
            let resized = self.resize_frame(frame)?;
            frame_buffer.push((timestamp, resized));
            scanned_duration = timestamp;

            // Send progress update periodically
            if frame_buffer.len() % 10 == 0 {
                self.send_progress(video_id, scanned_duration, total_duration, segments_found, explicit_segments).await;
            }

            // Process grid when we have enough frames
            if frame_buffer.len() >= frames_per_grid {
                let grid_frames: Vec<_> = frame_buffer.drain(..frames_per_grid).collect();
                let grid_start_time = grid_frames[0].0;

                match self.process_grid(video_id, grid_index, grid_start_time, grid_frames).await {
                    Ok(found_explicit) => {
                        segments_found += frames_per_grid;
                        explicit_segments += found_explicit;
                    }
                    Err(e) => {
                        error!("Grid processing failed: {}", e);
                        let mut stats = self.stats.write().await;
                        stats.api_errors += 1;
                    }
                }
                grid_index += 1;

                // Update scan state
                {
                    let mut scans = self.active_scans.write().await;
                    if let Some(state) = scans.get_mut(&video_id) {
                        state.scanned_duration = scanned_duration;
                        state.segments_found = segments_found;
                        state.explicit_segments = explicit_segments;
                    }
                }
            }
        }

        // Process remaining frames
        if !frame_buffer.is_empty() {
            let grid_start_time = frame_buffer[0].0;
            let _ = self.process_grid(video_id, grid_index, grid_start_time, frame_buffer).await;
        }

        // Final progress update
        self.send_progress(video_id, total_duration, total_duration, segments_found, explicit_segments).await;

        // Clean up
        self.active_scans.write().await.remove(&video_id);

        // Update global stats
        {
            let mut stats = self.stats.write().await;
            stats.videos_processed += 1;
            stats.total_segments_scanned += segments_found as u64;
            stats.explicit_segments_found += explicit_segments as u64;
        }

        info!("Content moderation complete for video {}: {} segments, {} explicit", 
            video_id, segments_found, explicit_segments);

        Ok(())
    }

    async fn process_grid(
        &self,
        video_id: VideoId,
        grid_index: u32,
        start_time: Duration,
        frames: Vec<(Duration, image::DynamicImage)>,
    ) -> Result<usize> {
        // Create grid image
        let grid_image = self.grid_processor.create_grid(&frames)?;

        // Send to Gemini
        let request = otip_core::domain::GridScanRequest {
            video_id,
            grid_index,
            start_time,
            frame_data: grid_image,
            mime_type: "image/png".to_string(),
        };

        let response = self.gemini_client.analyze_grid(&request).await?;

        // Process response
        let mut explicit_count = 0;
        
        for &quadrant in &response.explicit_quadrants {
            let segment_start = start_time + Duration::from_secs((quadrant - 1) as u64);
            let segment_end = segment_start + Duration::from_secs(1);

            let segment = TimelineSegment {
                start_time: segment_start,
                end_time: segment_end,
                segment_type: TimelineSegmentType::ExplicitContent,
                scan_segment: Some(ScanSegment {
                    start_time: segment_start,
                    end_time: segment_end,
                    is_explicit: true,
                    confidence: 0.9,
                    quadrant_flags: Some(QuadrantFlags::from_quadrant_numbers(&[quadrant])),
                }),
            };

            let _ = self.segment_tx.send((video_id, segment));
            explicit_count += 1;

            let mut stats = self.stats.write().await;
            stats.explicit_segments_found += 1;
        }

        // Send safe segments for non-flagged quadrants
        for q in 1..=4 {
            if !response.explicit_quadrants.contains(&q) {
                let segment_start = start_time + Duration::from_secs((q - 1) as u64);
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

                let _ = self.segment_tx.send((video_id, segment));
            }
        }

        let mut stats = self.stats.write().await;
        stats.api_calls += 1;
        stats.total_segments_scanned += frames.len() as u64;

        Ok(explicit_count)
    }

    fn resize_frame(&self, frame: image::DynamicImage) -> Result<image::DynamicImage> {
        let (w, h) = self.config.scanner_config.frame_resolution;
        Ok(frame.resize_exact(w, h, image::imageops::FilterType::Lanczos3))
    }

    async fn send_progress(
        &self,
        video_id: VideoId,
        scanned: Duration,
        total: Duration,
        segments_found: usize,
        explicit_segments: usize,
    ) {
        let _ = self.progress_tx.send(ScanProgress {
            video_id,
            scanned_duration: scanned,
            total_duration: total,
            segments_found,
            explicit_segments,
            current_position: scanned,
            is_complete: scanned >= total,
            error: None,
        });
    }

    /// Cancel an ongoing scan
    pub async fn cancel_scan(&self, video_id: VideoId) {
        let mut scans = self.active_scans.write().await;
        if let Some(state) = scans.get_mut(&video_id) {
            state.is_cancelled = true;
        }
    }

    /// Check if a scan is active
    pub async fn is_scanning(&self, video_id: VideoId) -> bool {
        self.active_scans.read().await.contains_key(&video_id)
    }

    /// Get moderator statistics
    pub async fn get_stats(&self) -> ModeratorStats {
        self.stats.read().await.clone()
    }

    /// Get Gemini client stats
    pub async fn get_gemini_stats(&self) -> crate::gemini::GeminiStats {
        self.gemini_client.get_stats().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_moderation_config_default() {
        let config = ModerationConfig::default();
        assert_eq!(config.auto_skip_enabled, true);
        assert_eq!(config.skip_buffer_ms, 500);
        assert_eq!(config.confidence_threshold, 0.7);
    }
}