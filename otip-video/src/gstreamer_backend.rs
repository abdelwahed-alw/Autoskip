//! GStreamer backend with AppSink for frame extraction

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use async_trait::async_trait;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use otip_core::domain::{VideoId, VideoMetadata, PlaybackState};
use otip_core::error::{Result, OtipError, VideoError};
use otip_core::events::VideoEngineResponse;
use image::{DynamicImage, ImageBuffer, Rgb, Rgba, ImageEncoder};
use image::codecs::png::PngEncoder;
use tracing::{debug, info, warn, error};
use crate::engine::EngineConfig;

/// Video engine trait - implemented by backends
#[async_trait]
pub trait VideoEngine: Send + Sync {
    /// Initialize the engine with a video file
    async fn initialize(&mut self, video_id: VideoId, path: &str) -> Result<VideoMetadata>;
    
    /// Start playback
    async fn play(&mut self, video_id: VideoId) -> Result<()>;
    
    /// Pause playback
    async fn pause(&mut self, video_id: VideoId) -> Result<()>;
    
    /// Stop playback
    async fn stop(&mut self, video_id: VideoId) -> Result<()>;
    
    /// Seek to position
    async fn seek(&mut self, video_id: VideoId, position: Duration) -> Result<()>;
    
    /// Set volume (0.0 - 1.0)
    async fn set_volume(&mut self, video_id: VideoId, volume: f32) -> Result<()>;
    
    /// Set playback rate
    async fn set_rate(&mut self, video_id: VideoId, rate: f32) -> Result<()>;
    
    /// Get current position and duration
    async fn get_position(&self, video_id: VideoId) -> Result<(Duration, Duration)>;
    
    /// Get current playback state
    async fn get_state(&self, video_id: VideoId) -> Result<PlaybackState>;
    
    /// Request a frame at specific timestamp (for scanning)
    async fn request_frame(&mut self, video_id: VideoId, timestamp: Duration) -> Result<DynamicImage>;
    
    /// Check if hardware acceleration is available
    fn hw_acceleration_available(&self) -> bool;
    
    /// Get engine type identifier
    fn engine_type(&self) -> crate::engine::EngineType;
    
    /// Shutdown the engine
    async fn shutdown(&mut self, video_id: VideoId) -> Result<()>;
}

/// GStreamer video engine implementation
pub struct GStreamerEngine {
    config: EngineConfig,
    instances: Arc<RwLock<HashMap<VideoId, GstInstance>>>,
    event_tx: Option<mpsc::UnboundedSender<VideoEngineResponse>>,
}

struct GstInstance {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    metadata: VideoMetadata,
    state: PlaybackState,
    frame_tx: mpsc::UnboundedSender<DynamicImage>,
}

impl GStreamerEngine {
    pub fn new() -> Self {
        gst::init().expect("Failed to initialize GStreamer");
        
        Self {
            config: EngineConfig::default(),
            instances: Arc::new(RwLock::new(HashMap::new())),
            event_tx: None,
        }
    }

    pub fn with_config(config: EngineConfig) -> Self {
        gst::init().expect("Failed to initialize GStreamer");
        
        Self {
            config,
            instances: Arc::new(RwLock::new(HashMap::new())),
            event_tx: None,
        }
    }

    fn create_pipeline(&self, path: &str) -> Result<(gst::Pipeline, gst_app::AppSink, mpsc::UnboundedSender<DynamicImage>)> {
        let pipeline = gst::Pipeline::new();
        
        // Create elements
        let uridecodebin = gst::ElementFactory::make("uridecodebin")
            .name("source")
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create uridecodebin".to_string())))?;
        
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .name("videoconvert")
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create videoconvert".to_string())))?;
        
        let videoscale = gst::ElementFactory::make("videoscale")
            .name("videoscale")
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create videoscale".to_string())))?;
        
        let appsink = gst::ElementFactory::make("appsink")
            .name("framesink")
            .property("emit-signals", true)
            .property("sync", false)
            .property("max-buffers", 1u32)
            .property("drop", true)
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create appsink".to_string())))?;
        
        let appsink = appsink.dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to cast appsink".to_string())))?;

        // Configure appsink caps for RGBA frame extraction
        let (w, h) = self.config.frame_extraction_resolution;
        let caps = gst_video::VideoCapsBuilder::new()
            .format(gst_video::VideoFormat::Rgba)
            .width(w as i32)
            .height(h as i32)
            .build();
        appsink.set_caps(Some(&caps));

        // Add elements to pipeline
        pipeline.add_many(&[&uridecodebin, &videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to add elements to pipeline".to_string())))?;

        // Link videoconvert -> videoscale -> appsink
        gst::Element::link_many(&[&videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to link video chain".to_string())))?;

        // Connect to uridecodebin pad-added signal
        let videoconvert_clone = videoconvert.clone();
        uridecodebin.connect_pad_added(move |_, src_pad| {
            let sink_pad = videoconvert_clone.static_pad("sink").unwrap();
            if src_pad.current_caps().unwrap().structure(0).unwrap().name().starts_with("video/") {
                let _ = src_pad.link(&sink_pad);
            }
        });

        // Set URI
        let uri = format!("file://{}", Path::new(path).canonicalize().unwrap().display());
        uridecodebin.set_property("uri", &uri);

        // Frame extraction channel
        let (frame_tx, _frame_rx) = mpsc::unbounded_channel::<DynamicImage>();

        // Configure appsink callbacks for frame extraction
        let appsink_clone = appsink.clone();
        let frame_tx_clone = frame_tx.clone();
        
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    // In gstreamer 0.21, pull_sample returns Result<Sample, FlowError>
                    let sample = match appsink.pull_sample() {
                        Ok(s) => s,
                        Err(_) => return Err(gst::FlowError::Eos),
                    };
                    
                    let buffer = match sample.buffer() {
                        Some(b) => b,
                        None => return Err(gst::FlowError::Error),
                    };
                    
                    // map_readable returns Result<ReadableBuffer, MapError>
                    let map = match buffer.map_readable() {
                        Ok(m) => m,
                        Err(_) => return Err(gst::FlowError::Error),
                    };
                    
                    let caps = match sample.caps() {
                        Some(c) => c,
                        None => return Err(gst::FlowError::Error),
                    };
                    
                    let structure = match caps.structure(0) {
                        Some(s) => s,
                        None => return Err(gst::FlowError::Error),
                    };
                    
                    let width = structure.get::<i32>("width").unwrap_or(640) as u32;
                    let height = structure.get::<i32>("height").unwrap_or(360) as u32;
                    let data = map.as_slice();
                    
                    // Convert to DynamicImage (RGBA)
                    if data.len() >= (width * height * 4) as usize {
                        let img = ImageBuffer::from_raw(width, height, data.to_vec())
                            .map(DynamicImage::ImageRgba8);
                        if let Some(img) = img {
                            let _ = frame_tx_clone.send(img);
                        }
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        Ok((pipeline, appsink, frame_tx))
    }

    /// Extract a thumbnail frame at a specific timestamp (e.g., 5 seconds)
    /// This is used during video scanning to generate thumbnails.
    /// Pipeline: uridecodebin -> videoconvert -> videoscale -> appsink (Rgba 320x180)
    /// Caches frame in memory (VideoThumbnail) and optionally as lightweight temp file.
    pub async fn extract_thumbnail(path: &str, timestamp: Duration) -> Result<Option<otip_core::domain::VideoThumbnail>> {
        gst::init().ok();

        let uridecodebin = gst::ElementFactory::make("uridecodebin")
            .name("thumb_source")
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create uridecodebin".to_string())))?;

        let videoconvert = gst::ElementFactory::make("videoconvert")
            .name("thumb_convert")
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create videoconvert".to_string())))?;

        let videoscale = gst::ElementFactory::make("videoscale")
            .name("thumb_scale")
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create videoscale".to_string())))?;

        let appsink = gst::ElementFactory::make("appsink")
            .name("thumbnailsink")
            .property("emit-signals", true)
            .property("sync", false)
            .property("max-buffers", 1u32)
            .property("drop", true)
            .build()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to create appsink".to_string())))?;

        let appsink = appsink.dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to cast appsink".to_string())))?;

        let caps = gst_video::VideoCapsBuilder::new()
            .format(gst_video::VideoFormat::Rgba)
            .width(320) // Thumbnail size: 320x180 lightweight
            .height(180)
            .build();
        appsink.set_caps(Some(&caps));

        let pipeline = gst::Pipeline::new();
        pipeline.add_many(&[&uridecodebin, &videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to add elements to pipeline".to_string())))?;

        gst::Element::link_many(&[&videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to link video chain".to_string())))?;

        // Robust pad-added handling (check video/ caps)
        let vc_clone = videoconvert.clone();
        uridecodebin.connect_pad_added(move |_, src_pad| {
            // current_caps may be None during negotiation; fallback to query_caps
            let caps = src_pad
                .current_caps()
                .unwrap_or_else(|| src_pad.query_caps(None));
            if let Some(s) = caps.structure(0) {
                if s.name().starts_with("video/") {
                    if let Some(sink_pad) = vc_clone.static_pad("sink") {
                        let _ = src_pad.link(&sink_pad);
                    }
                }
            }
        });

        let uri = format!("file://{}", std::path::Path::new(path).canonicalize().unwrap_or_else(|_| std::path::Path::new(path).to_path_buf()).display());
        uridecodebin.set_property("uri", &uri);

        // Use a oneshot channel to get the thumbnail frame
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Need interior mutability for one-shot send (allow only first frame)
        let tx_cell = std::sync::Arc::new(parking_lot::Mutex::new(Some(tx)));

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                    let s = caps.structure(0).ok_or(gst::FlowError::Error)?;
                    let width = s.get::<i32>("width").unwrap_or(320) as u32;
                    let height = s.get::<i32>("height").unwrap_or(180) as u32;
                    let data = map.as_slice().to_vec();
                    if let Some(tx) = tx_cell.lock().take() {
                        let _ = tx.send((data, width, height));
                    }
                    Err(gst::FlowError::Eos) // we only need one frame
                })
                .build(),
        );

        // Start pipeline in Paused to preroll, then seek to 5s, then Playing
        pipeline.set_state(gst::State::Paused)
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to preroll pipeline".to_string())))?;
        // Wait briefly for PAUSED (up to 2s) to ensure uridecodebin linked
        let _ = pipeline.state(gst::ClockTime::from_seconds(2));

        // Seek to timestamp (default 5 seconds per spec)
        let seek_event = gst::event::Seek::new(
            1.0,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::SeekType::Set,
            gst::ClockTime::from_nseconds((timestamp.as_nanos() as u64).try_into().unwrap_or(0)),
            gst::SeekType::None,
            gst::ClockTime::NONE,
        );
        pipeline.send_event(seek_event);

        pipeline.set_state(gst::State::Playing)
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to start pipeline".to_string())))?;

        // Wait for frame with timeout
        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok((data, width, height))) => {
                pipeline.set_state(gst::State::Null).ok();

                // Encode as PNG for lightweight temp file cache + in-memory
                // Keep raw RGBA copy for encoding without move issues
                let raw_clone = data.clone();
                let img_buf: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, data)
                    .ok_or_else(|| OtipError::Video(VideoError::DecodeError("Failed to create image buffer".to_string())))?;

                let mut png_data = Vec::new();
                {
                    let encoder = PngEncoder::new(&mut png_data);
                    // Use the cloned raw data to avoid move conflict
                    encoder
                        .write_image(&raw_clone, width, height, image::ColorType::Rgba8.into())
                        .map_err(|e| OtipError::Video(VideoError::DecodeError(format!("PNG encode failed: {}", e))))?;
                    let _ = img_buf; // suppress unused warning if not needed
                }

                // Optionally save lightweight temp file for disk cache (non-fatal)
                {
                    let mut tmp = std::env::temp_dir().join("otip_thumbs");
                    let _ = std::fs::create_dir_all(&tmp);
                    let hash = {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        path.hash(&mut h);
                        format!("{:x}", h.finish())
                    };
                    tmp.push(format!("{}-{}x{}.png", hash, width, height));
                    let _ = std::fs::write(&tmp, &png_data);
                }

                Ok(Some(otip_core::domain::VideoThumbnail::new(png_data, width, height)))
            }
            Ok(Err(e)) => {
                pipeline.set_state(gst::State::Null).ok();
                Err(OtipError::Video(VideoError::DecodeError(format!("Thumbnail channel error: {}", e))))
            }
            Err(_) => {
                pipeline.set_state(gst::State::Null).ok();
                // Timeout - no frame (short video or unsupported). Return None gracefully; caller falls back to placeholder.
                Ok(None)
            }
        }
    }

    /// Convenience wrapper extracting at 5 seconds per spec
    pub async fn extract_thumbnail_at_5s(path: &str) -> Result<Option<otip_core::domain::VideoThumbnail>> {
        Self::extract_thumbnail(path, Duration::from_secs(5)).await
    }

}

#[async_trait]
impl crate::engine::VideoEngine for GStreamerEngine {
    async fn initialize(&mut self, video_id: VideoId, path: &str) -> Result<VideoMetadata> {
        info!("Initializing GStreamer for video {}: {}", video_id, path);

        let (pipeline, appsink, frame_tx) = self.create_pipeline(path)?;

        // Start pipeline
        pipeline.set_state(gst::State::Playing)
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to start pipeline".to_string())))?;

        // Wait for pipeline to preroll
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Get metadata from pipeline
        let mut width = 0;
        let mut height = 0;
        let mut fps = 0.0;
        let mut duration = Duration::ZERO;
        let mut codec = String::new();
        let mut has_audio = false;

        // Query video info
        if let Some(video_sink) = pipeline.by_name("videoconvert") {
            if let Some(pad) = video_sink.static_pad("src") {
                if let Some(caps) = pad.current_caps() {
                    if let Some(structure) = caps.structure(0) {
                        width = structure.get::<i32>("width").unwrap_or(0) as u32;
                        height = structure.get::<i32>("height").unwrap_or(0) as u32;
                        if let Ok(fraction) = structure.get::<gst::Fraction>("framerate") {
                            fps = fraction.numer() as f32 / fraction.denom() as f32;
                        }
                    }
                }
            }
        }

        // Query duration
        if let Some(dur) = pipeline.query_duration::<gst::ClockTime>() {
            duration = Duration::from_nanos(dur.nseconds() as u64);
        }

        let metadata = VideoMetadata {
            id: video_id,
            path: path.to_string(),
            title: Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string(),
            duration,
            width,
            height,
            fps,
            codec,
            has_audio,
            created_at: chrono::Utc::now(),
            last_played: None,
            thumbnail: None,
        };

        let instance = GstInstance {
            pipeline,
            appsink,
            metadata: metadata.clone(),
            state: PlaybackState::Playing,
            frame_tx,
        };

        self.instances.write().await.insert(video_id, instance);

        Ok(metadata)
    }

    async fn play(&mut self, video_id: VideoId) -> Result<()> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            instance.pipeline.set_state(gst::State::Playing)
                .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to play".to_string())))?;
            Ok(())
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn pause(&mut self, video_id: VideoId) -> Result<()> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            instance.pipeline.set_state(gst::State::Paused)
                .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to pause".to_string())))?;
            Ok(())
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn stop(&mut self, video_id: VideoId) -> Result<()> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            instance.pipeline.set_state(gst::State::Null)
                .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to stop".to_string())))?;
            Ok(())
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn seek(&mut self, video_id: VideoId, position: Duration) -> Result<()> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            let seek_event = gst::event::Seek::new(
                1.0,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                gst::ClockTime::from_nseconds((position.as_nanos() as u64).try_into().unwrap_or(u64::MAX)),
                gst::SeekType::None,
                gst::ClockTime::NONE,
            );
            instance.pipeline.send_event(seek_event);
            Ok(())
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn set_volume(&mut self, video_id: VideoId, volume: f32) -> Result<()> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            if let Some(volume_elem) = instance.pipeline.by_name("volume") {
                volume_elem.set_property("volume", volume as f64);
            }
            Ok(())
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn set_rate(&mut self, video_id: VideoId, rate: f32) -> Result<()> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            let seek_event = gst::event::Seek::new(
                rate as f64,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                gst::ClockTime::from_nseconds(0),
                gst::SeekType::None,
                gst::ClockTime::NONE,
            );
            instance.pipeline.send_event(seek_event);
            Ok(())
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn get_position(&self, video_id: VideoId) -> Result<(Duration, Duration)> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            let position = instance.pipeline.query_position::<gst::ClockTime>()
                .map(|t| Duration::from_nanos(t.nseconds() as u64))
                .unwrap_or(Duration::ZERO);
            let duration = instance.pipeline.query_duration::<gst::ClockTime>()
                .map(|t| Duration::from_nanos(t.nseconds() as u64))
                .unwrap_or(instance.metadata.duration);
            Ok((position, duration))
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn get_state(&self, video_id: VideoId) -> Result<PlaybackState> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            Ok(instance.state)
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    async fn request_frame(&mut self, video_id: VideoId, _timestamp: Duration) -> Result<DynamicImage> {
        let (w, h) = self.config.frame_extraction_resolution;
        let mut img = ImageBuffer::new(w, h);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = Rgb([(x % 255) as u8, (y % 255) as u8, 128]);
        }
        info!("Extracted frame {}x{} (GStreamer appsink placeholder)", w, h);
        Ok(DynamicImage::ImageRgb8(img))
    }

    fn hw_acceleration_available(&self) -> bool {
        gst::ElementFactory::find("vaapidecode").is_some() ||
        gst::ElementFactory::find("d3d11h264dec").is_some() ||
        gst::ElementFactory::find("vtdec").is_some()
    }

    fn engine_type(&self) -> crate::engine::EngineType {
        crate::engine::EngineType::GStreamer
    }

    async fn shutdown(&mut self, video_id: VideoId) -> Result<()> {
        let mut instances = self.instances.write().await;
        if let Some(mut instance) = instances.remove(&video_id) {
            instance.pipeline.set_state(gst::State::Null).ok();
            Ok(())
        } else {
            Ok(())
        }
    }
}

impl Default for GStreamerEngine {
    fn default() -> Self {
        Self::new()
    }
}
