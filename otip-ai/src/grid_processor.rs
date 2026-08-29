//! Grid image processor for batching frames

use std::sync::Arc;
use image::{DynamicImage, ImageBuffer, Rgb};
use otip_core::error::{Result, OtipError, ScannerError};

/// Configuration for grid processing
#[derive(Debug, Clone)]
pub struct GridConfig {
    pub grid_size: (u32, u32),      // rows, cols (e.g., 2x2)
    pub frame_resolution: (u32, u32), // per-frame resolution
    pub output_format: ImageFormat,
    pub quality: u8, // for JPEG
}

impl Default for GridConfig {
    fn default() -> Self {
        Self {
            grid_size: (2, 2),
            frame_resolution: (320, 240),
            output_format: ImageFormat::Png,
            quality: 85,
        }
    }
}

/// Output image format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

/// Grid processor for creating composite images
pub struct GridProcessor {
    config: GridConfig,
}

impl GridProcessor {
    pub fn new(config: GridConfig) -> Self {
        Self { config }
    }

    /// Create a grid image from multiple frames
    pub fn create_grid(&self, frames: &[(std::time::Duration, DynamicImage)]) -> Result<Vec<u8>> {
        let (grid_rows, grid_cols) = self.config.grid_size;
        let (frame_w, frame_h) = self.config.frame_resolution;
        
        let grid_width = grid_cols * frame_w;
        let grid_height = grid_rows * frame_h;
        
        let mut grid = ImageBuffer::new(grid_width, grid_height);

        for (idx, (_, frame)) in frames.iter().enumerate() {
            if idx >= (grid_rows * grid_cols) as usize {
                break;
            }
            
            let row = (idx as u32) / grid_cols;
            let col = (idx as u32) % grid_cols;
            let x = col * frame_w;
            let y = row * frame_h;

            // Convert to RGB if needed
            let rgb_frame = frame.to_rgb8();
            
            for fy in 0..frame_h.min(rgb_frame.height()) {
                for fx in 0..frame_w.min(rgb_frame.width()) {
                    let pixel = rgb_frame.get_pixel(fx, fy);
                    grid.put_pixel(x + fx, y + fy, *pixel);
                }
            }
        }

        // Encode to bytes
        let mut bytes = Vec::new();
        match self.config.output_format {
            ImageFormat::Png => {
                grid.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                    .map_err(|e| OtipError::Scanner(ScannerError::GridCreationFailed(e.to_string())))?;
            }
            ImageFormat::Jpeg => {
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, self.config.quality);
                encoder.encode_image(&grid)
                    .map_err(|e| OtipError::Scanner(ScannerError::GridCreationFailed(e.to_string())))?;
            }
        }

        Ok(bytes)
    }

    /// Create grid from raw frame buffers (more efficient)
    pub fn create_grid_from_buffers(&self, frames: &[&[u8]]) -> Result<Vec<u8>> {
        let (grid_rows, grid_cols) = self.config.grid_size;
        let (frame_w, frame_h) = self.config.frame_resolution;
        let frame_size = (frame_w * frame_h * 3) as usize;
        
        let grid_width = grid_cols * frame_w;
        let grid_height = grid_rows * frame_h;
        
        let mut grid = ImageBuffer::new(grid_width, grid_height);

        for (idx, frame_data) in frames.iter().enumerate() {
            if idx >= (grid_rows * grid_cols) as usize {
                break;
            }
            
            if frame_data.len() < frame_size {
                continue;
            }

            let row = (idx as u32) / grid_cols;
            let col = (idx as u32) % grid_cols;
            let x = col * frame_w;
            let y = row * frame_h;

            for fy in 0..frame_h {
                for fx in 0..frame_w {
                    let src_idx = ((fy * frame_w + fx) * 3) as usize;
                    let dst_x = x + fx;
                    let dst_y = y + fy;
                    
                    if src_idx + 2 < frame_data.len() {
                        grid.put_pixel(dst_x, dst_y, Rgb([
                            frame_data[src_idx],
                            frame_data[src_idx + 1],
                            frame_data[src_idx + 2],
                        ]));
                    }
                }
            }
        }

        let mut bytes = Vec::new();
        match self.config.output_format {
            ImageFormat::Png => {
                grid.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                    .map_err(|e| OtipError::Scanner(ScannerError::GridCreationFailed(e.to_string())))?;
            }
            ImageFormat::Jpeg => {
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, self.config.quality);
                encoder.encode_image(&grid)
                    .map_err(|e| OtipError::Scanner(ScannerError::GridCreationFailed(e.to_string())))?;
            }
        }

        Ok(bytes)
    }

    /// Calculate grid dimensions
    pub fn grid_dimensions(&self) -> (u32, u32) {
        let (grid_rows, grid_cols) = self.config.grid_size;
        let (frame_w, frame_h) = self.config.frame_resolution;
        (grid_cols * frame_w, grid_rows * frame_h)
    }

    /// Get frames per grid
    pub fn frames_per_grid(&self) -> usize {
        (self.config.grid_size.0 * self.config.grid_size.1) as usize
    }

    /// Update configuration
    pub fn set_config(&mut self, config: GridConfig) {
        self.config = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn test_create_grid() {
        let config = GridConfig::default();
        let processor = GridProcessor::new(config);

        // Create 4 test frames with different colors
        let frames: Vec<_> = (0..4).map(|i| {
            let mut img = ImageBuffer::new(320, 240);
            for (x, y, pixel) in img.enumerate_pixels_mut() {
                *pixel = Rgb([(i * 50) as u8, (i * 30) as u8, (i * 10) as u8]);
            }
            (Duration::from_secs(i), DynamicImage::ImageRgb8(img))
        }).collect();

        let result = processor.create_grid(&frames);
        assert!(result.is_ok());
        
        let grid_bytes = result.unwrap();
        assert!(!grid_bytes.is_empty());
        
        // Verify dimensions
        let grid_img = image::load_from_memory(&grid_bytes).unwrap();
        assert_eq!(grid_img.width(), 640); // 2 * 320
        assert_eq!(grid_img.height(), 480); // 2 * 240
    }

    #[test]
    fn test_grid_dimensions() {
        let config = GridConfig {
            grid_size: (2, 2),
            frame_resolution: (320, 240),
            ..Default::default()
        };
        let processor = GridProcessor::new(config);
        assert_eq!(processor.grid_dimensions(), (640, 480));
        assert_eq!(processor.frames_per_grid(), 4);
    }

    #[test]
    fn test_create_grid_from_buffers() {
        let config = GridConfig::default();
        let processor = GridProcessor::new(config);

        // Create 4 test frame buffers (RGB)
        let frame_size = 320 * 240 * 3;
        let frames: Vec<Vec<u8>> = (0..4).map(|i| {
            let mut buf = vec![0u8; frame_size];
            for chunk in buf.chunks_exact_mut(3) {
                chunk[0] = (i * 50) as u8;
                chunk[1] = (i * 30) as u8;
                chunk[2] = (i * 10) as u8;
            }
            buf
        }).collect();

        let frame_refs: Vec<&[u8]> = frames.iter().map(|v| v.as_slice()).collect();
        let result = processor.create_grid_from_buffers(&frame_refs);
        assert!(result.is_ok());
        
        let grid_bytes = result.unwrap();
        assert!(!grid_bytes.is_empty());
    }
}