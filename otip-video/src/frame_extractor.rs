//! Batch frame extraction for scanning

use crate::engine::VideoEngine;
use image::DynamicImage;
use otip_core::error::Result;
use otip_core::domain::VideoId;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// High-performance frame extractor using GStreamer appsink
#[cfg(feature = "gstreamer")]
pub struct MpvFrameExtractor {
    engine: Arc<tokio::sync::Mutex<crate::gstreamer_backend::GStreamerEngine>>,
    interval: Duration,
    resolution: (u32, u32),
}

#[cfg(feature = "gstreamer")]
impl MpvFrameExtractor {
    pub fn new(
        engine: Arc<tokio::sync::Mutex<crate::gstreamer_backend::GStreamerEngine>>,
        interval: Duration,
        resolution: (u32, u32),
    ) -> Self {
        Self {
            engine,
            interval,
            resolution,
        }
    }

    pub async fn start_extraction(
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

            let frame = Self::extract_frame_at(&engine, current, self.resolution).await?;
            if tx.send((current, frame)).is_err() {
                break;
            }

            current += self.interval;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    async fn extract_frame_at(
        engine: &Arc<tokio::sync::Mutex<crate::gstreamer_backend::GStreamerEngine>>,
        timestamp: Duration,
        _resolution: (u32, u32),
    ) -> Result<DynamicImage> {
        let engine = engine.lock().await;
        // This will be implemented by the GStreamer backend
        engine.request_frame(VideoId::new(), timestamp).await
    }
}