//! Batch frame extraction for scanning
//!
//! This module provides frame extraction abstractions used by the scanning
//! pipeline. The primary extraction path now uses ffmpeg via `otip-core::scan`
//! (`extract_frames_1fps`). This module retains the trait and placeholder for
//! future mpv render-context based extraction.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use image::DynamicImage;
use otip_core::domain::VideoId;
use otip_core::error::Result;

/// Grid frame extractor — extracts frames in batches suitable for AI grid
/// compositing. Current implementation delegates to the video engine.
pub struct GridFrameExtractor {
    engine: Arc<tokio::sync::Mutex<Box<dyn crate::engine::VideoEngine>>>,
    interval: Duration,
    resolution: (u32, u32),
    grid_size: (u32, u32),
}

impl GridFrameExtractor {
    pub fn new(
        engine: Arc<tokio::sync::Mutex<Box<dyn crate::engine::VideoEngine>>>,
        interval: Duration,
        resolution: (u32, u32),
        grid_size: (u32, u32),
    ) -> Self {
        Self {
            engine,
            interval,
            resolution,
            grid_size,
        }
    }

    /// Extract frames at the configured interval across the video duration.
    /// Sends `(timestamp, frame)` pairs to `tx`. Stops when the duration is
    /// reached or the receiver is dropped.
    pub async fn extract_grids(
        &self,
        video_id: VideoId,
        duration: Duration,
        tx: mpsc::UnboundedSender<(Duration, DynamicImage)>,
    ) -> Result<()> {
        let mut current = Duration::ZERO;
        let engine = self.engine.clone();

        while current < duration {
            if tx.is_closed() {
                break;
            }

            match engine.lock().await.request_frame(video_id, current).await {
                Ok(frame) => {
                    if tx.send((current, frame)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!("Frame extraction failed at {:?}: {}", current, e);
                    // Skip this frame, continue with next
                }
            }

            current += self.interval;
            // Small yield to avoid starving other tasks
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }
}
