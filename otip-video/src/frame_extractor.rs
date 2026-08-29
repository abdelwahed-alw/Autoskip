//! Frame extraction for scanning

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use async_trait::async_trait;
use otip_core::domain::VideoId;
use otip_core::error::Result;
use image::DynamicImage;
use crate::engine::VideoEngine;

/// Frame extractor that uses the video engine to extract frames at intervals
pub struct FrameExtractor<E: VideoEngine> {
    engine: Arc<tokio::sync::Mutex<E>>,
    interval: Duration,
    resolution: (u32, u32),
}

impl<E: VideoEngine> FrameExtractor<E> {
    pub fn new(engine: Arc<tokio::sync::Mutex<E>>, interval: Duration, resolution: (u32, u32)) -> Self {
        Self {
            engine,
            interval,
            resolution,
        }
    }

    /// Start extracting frames and send them through the channel
    pub async fn start_extraction(
        &self,
        video_id: VideoId,
        duration: Duration,
        tx: mpsc::UnboundedSender<(Duration, DynamicImage)>,
    ) -> Result<()> {
        let mut current = Duration::ZERO;
        let engine = self.engine.clone();
        
        while current < duration {
            // Check if channel is still open
            if tx.is_closed() {
                break;
            }

            let frame = engine.lock().await.request_frame(video_id, current).await?;
            
            if tx.send((current, frame)).is_err() {
                break; // Receiver dropped
            }

            current += self.interval;
            
            // Small delay to not overwhelm the engine
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }
}

/// High-performance frame extractor using MPV's built-in screenshot
#[cfg(feature = "mpv")]
pub struct MpvFrameExtractor {
    mpv: Arc<tokio::sync::Mutex<crate::mpv_backend::MpvHandle>>,
    interval: Duration,
    resolution: (u32, u32),
}

#[cfg(feature = "mpv")]
impl MpvFrameExtractor {
    pub fn new(mpv: Arc<tokio::sync::Mutex<crate::mpv_backend::MpvHandle>>, interval: Duration, resolution: (u32, u32)) -> Self {
        Self { mpv, interval, resolution }
    }

    pub async fn start_extraction(
        &self,
        video_id: VideoId,
        duration: Duration,
        tx: mpsc::UnboundedSender<(Duration, DynamicImage)>,
    ) -> Result<()> {
        let mut current = Duration::ZERO;
        let mpv = self.mpv.clone();
        let resolution = self.resolution;
        
        while current < duration {
            if tx.is_closed() {
                break;
            }

            let frame = Self::extract_frame_at(&mpv, current, resolution).await?;
            
            if tx.send((current, frame)).is_err() {
                break;
            }

            current += self.interval;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    async fn extract_frame_at(
        _mpv: &Arc<tokio::sync::Mutex<crate::mpv_backend::MpvHandle>>,
        _timestamp: Duration,
        resolution: (u32, u32),
    ) -> Result<DynamicImage> {
        // Stub implementation
        let (w, h) = resolution;
        let mut img = image::ImageBuffer::new(w, h);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x % 255) as u8, (y % 255) as u8, 128]);
        }
        Ok(DynamicImage::ImageRgb8(img))
    }
}

/// Batch frame extractor for grid processing
pub struct GridFrameExtractor<E: VideoEngine> {
    extractor: FrameExtractor<E>,
    grid_size: (u32, u32),
}

impl<E: VideoEngine> GridFrameExtractor<E> {
    pub fn new(engine: Arc<tokio::sync::Mutex<E>>, interval: Duration, resolution: (u32, u32), grid_size: (u32, u32)) -> Self {
        Self {
            extractor: FrameExtractor::new(engine, interval, resolution),
            grid_size,
        }
    }

    /// Extract frames in batches for grid processing
    pub async fn extract_grids(
        &self,
        video_id: VideoId,
        duration: Duration,
        tx: mpsc::UnboundedSender<(u32, Duration, Vec<(Duration, DynamicImage)>)>,
    ) -> Result<()> {
        let frames_per_grid = (self.grid_size.0 * self.grid_size.1) as usize;
        let mut current = Duration::ZERO;
        let mut grid_index = 0u32;
        let mut frame_buffer = Vec::new();
        let engine = self.extractor.engine.clone();

        while current < duration {
            if tx.is_closed() {
                break;
            }

            let frame = engine.lock().await.request_frame(video_id, current).await?;
            frame_buffer.push((current, frame));

            if frame_buffer.len() >= frames_per_grid {
                let grid_start = frame_buffer[0].0;
                let grid_frames: Vec<_> = frame_buffer.drain(..frames_per_grid).collect();
                
                if tx.send((grid_index, grid_start, grid_frames)).is_err() {
                    break;
                }
                grid_index += 1;
            }

            current += self.extractor.interval;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Send remaining frames
        if !frame_buffer.is_empty() {
            let grid_start = frame_buffer[0].0;
            let _ = tx.send((grid_index, grid_start, frame_buffer));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use otip_core::domain::VideoId;

    #[test]
    fn test_grid_calculation() {
        let grid_size = (2, 2);
        let frames_per_grid = (grid_size.0 * grid_size.1) as usize;
        assert_eq!(frames_per_grid, 4);
    }
}