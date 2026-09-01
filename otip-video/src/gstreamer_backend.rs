//! GStreamer backend implementation

#[cfg(feature = "gstreamer")]
use std::collections::HashMap;
#[cfg(feature = "gstreamer")]
use std::sync::Arc;
#[cfg(feature = "gstreamer")]
use std::time::Duration;
#[cfg(feature = "gstreamer")]
use tokio::sync::{mpsc, RwLock, Mutex};
#[cfg(feature = "gstreamer")]
use async_trait::async_trait;
#[cfg(feature = "gstreamer")]
use gstreamer as gst;
#[cfg(feature = "gstreamer")]
use gstreamer::prelude::*;
#[cfg(feature = "gstreamer")]
use gstreamer_app as gst_app;
#[cfg(feature = "gstreamer")]
use gstreamer_video as gst_video;
#[cfg(feature = "gstreamer")]
use otip_core::domain::{VideoId, VideoMetadata, PlaybackState};
#[cfg(feature = "gstreamer")]
use otip_core::error::{Result, OtipError, VideoError};
#[cfg(feature = "gstreamer")]
use otip_core::events::{VideoEngineEvent, VideoEngineResponse};
#[cfg(feature = "gstreamer")]
use image::{DynamicImage, ImageBuffer, Rgb};
#[cfg(feature = "gstreamer")]
use tracing::{debug, info, warn, error};

#[cfg(feature = "gstreamer")]
struct GstInstance {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    metadata: VideoMetadata,
    state: PlaybackState,
    frame_request_tx: mpsc::UnboundedSender<(Duration, mpsc::UnboundedSender<Result<DynamicImage>>)>,
    bus_watch: Option<gst::BusWatch>,
}

#[cfg(feature = "gstreamer")]
pub struct GStreamerEngine {
    config: crate::EngineConfig,
    instances: Arc<RwLock<HashMap<VideoId, GstInstance>>>,
    event_tx: Option<mpsc::UnboundedSender<VideoEngineResponse>>,
    #[allow(dead_code)]
    context: gst::MainContext,
}

#[cfg(feature = "gstreamer")]
impl GStreamerEngine {
    pub fn new() -> Self {
        gst::init().expect("Failed to initialize GStreamer");
        
        Self {
            config: crate::EngineConfig::default(),
            instances: Arc::new(RwLock::new(HashMap::new())),
            event_tx: None,
            context: gst::MainContext::default(),
        }
    }

    pub fn with_config(config: crate::EngineConfig) -> Self {
        gst::init().expect("Failed to initialize GStreamer");
        
        Self {
            config,
            instances: Arc::new(RwLock::new(HashMap::new())),
            event_tx: None,
            context: gst::MainContext::default(),
        }
    }

    fn create_pipeline(&self, video_id: VideoId, path: &str) -> Result<(gst::Pipeline, gst_app::AppSink, mpsc::UnboundedSender<(Duration, mpsc::UnboundedSender<Result<DynamicImage>>)>)> {
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
        let uri = format!("file://{}", std::path::Path::new(path).canonicalize().unwrap().display());
        uridecodebin.set_property("uri", &uri);

        // Frame extraction channel
        let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<(Duration, mpsc::UnboundedSender<Result<DynamicImage>>)>();
        
        let appsink_clone = appsink.clone();
        let pipeline_clone = pipeline.clone();
        let frame_resolution = self.config.frame_extraction_resolution;
        
        let frame_task = tokio::spawn(async move {
            while let Some((timestamp, response_tx)) = frame_rx.recv().await {
                let result = Self::extract_frame_gst(&pipeline_clone, &appsink_clone, timestamp, frame_resolution).await;
                let _ = response_tx.send(result);
            }
        });

        Ok((pipeline, appsink, frame_tx))
    }

    async fn extract_frame_gst(
        pipeline: &gst::Pipeline,
        appsink: &gst_app::AppSink,
        timestamp: Duration,
        resolution: (u32, u32),
    ) -> Result<DynamicImage> {
        // Seek to timestamp
        let seek_event = gst::event::Seek::new(
            1.0,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::SeekType::Set,
            gst::ClockTime::from_nseconds((timestamp.as_nanos() as u64).try_into().unwrap_or(u64::MAX)),
            gst::SeekType::None,
            gst::ClockTime::NONE,
        );
        pipeline.send_event(seek_event);

        // Wait for seek to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Pull sample from appsink
        match appsink.pull_sample() {
            Some(sample) => {
                let buffer = sample.buffer().ok_or_else(|| OtipError::Video(VideoError::DecodeError("No buffer in sample".to_string())))?;
                let map = buffer.map_readable().map_err(|_| OtipError::Video(VideoError::DecodeError("Failed to map buffer".to_string())))?;
                
                let (w, h) = resolution;
                let data = map.as_slice();
                let expected_size = (w * h * 3) as usize;
                
                if data.len() >= expected_size {
                    let mut img = ImageBuffer::new(w, h);
                    for y in 0..h {
                        for x in 0..w {
                            let idx = ((y * w + x) * 3) as usize;
                            if idx + 2 < data.len() {
                                img.put_pixel(x, y, Rgb([data[idx], data[idx + 1], data[idx + 2]]));
                            }
                        }
                    }
                    Ok(DynamicImage::ImageRgb8(img))
                } else {
                    Err(OtipError::Video(VideoError::DecodeError("Buffer too small".to_string())))
                }
            }
            None => Err(OtipError::Video(VideoError::DecodeError("No sample available".to_string()))),
        }
    }

    fn setup_bus_watch(
        pipeline: &gst::Pipeline,
        video_id: VideoId,
        event_tx: mpsc::UnboundedSender<VideoEngineResponse>,
    ) -> gst::BusWatch {
        let bus = pipeline.bus().unwrap();
        let video_id_clone = video_id;
        let event_tx_clone = event_tx.clone();
        
        bus.add_watch(move |_, msg| {
            use gst::MessageView;
            
            match msg.view() {
                MessageView::Eos(..) => {
                    let _ = event_tx_clone.send(VideoEngineResponse::EndOfFile(video_id_clone));
                }
                MessageView::StateChanged(state_changed) => {
                    if state_changed.src().map(|s| s == pipeline).unwrap_or(false) {
                        let (_, new, _) = state_changed.states();
                        let state = match new {
                            gst::State::Playing => PlaybackState::Playing,
                            gst::State::Paused => PlaybackState::Paused,
                            gst::State::Ready | gst::State::Null => PlaybackState::Stopped,
                            _ => PlaybackState::Buffering,
                        };
                        let _ = event_tx_clone.send(VideoEngineResponse::StateChanged {
                            video_id: video_id_clone,
                            state,
                        });
                    }
                }
                MessageView::Error(err) => {
                    let _ = event_tx_clone.send(VideoEngineResponse::Error {
                        video_id: Some(video_id_clone),
                        error: format!("GStreamer error: {} - {}", err.error(), err.debug().unwrap_or_default()),
                    });
                }
                _ => {}
            }
            
            glib::ControlFlow::Continue
        })
    }
}

#[cfg(feature = "gstreamer")]
#[async_trait]
impl crate::engine::VideoEngine for GStreamerEngine {
    async fn initialize(&mut self, video_id: VideoId, path: &str) -> Result<VideoMetadata> {
        info!("Initializing GStreamer for video {}: {}", video_id, path);

        let (pipeline, appsink, frame_tx) = self.create_pipeline(video_id, path)?;

        // Start pipeline
        pipeline.set_state(gst::State::Playing)
            .map_err(|_| OtipError::Video(VideoError::InitFailed("Failed to start pipeline".to_string())))?;

        // Wait for pipeline to preroll
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Get metadata from pipeline
        let metadata = {
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
            if let Ok(Some(dur)) = pipeline.query_duration::<gst::ClockTime>() {
                duration = Duration::from_nanos(dur.nseconds());
            }

            VideoMetadata {
                id: video_id,
                path: path.to_string(),
                title: std::path::Path::new(path)
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
            }
        };

        let event_tx = self.event_tx.clone().expect("Event TX not set");
        let bus_watch = Self::setup_bus_watch(&pipeline, video_id, event_tx);

        let instance = GstInstance {
            pipeline,
            appsink,
            metadata: metadata.clone(),
            state: PlaybackState::Playing,
            frame_request_tx: frame_tx,
            bus_watch: Some(bus_watch),
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

    async fn request_frame(&mut self, video_id: VideoId, timestamp: Duration) -> Result<DynamicImage> {
        let instances = self.instances.read().await;
        if let Some(instance) = instances.get(&video_id) {
            let (response_tx, mut response_rx) = mpsc::unbounded_channel();
            instance.frame_request_tx.send((timestamp, response_tx))
                .map_err(|_| OtipError::Video(VideoError::DecodeError("Frame request channel closed".to_string())))?;
            
            response_rx.recv().await
                .ok_or_else(|| OtipError::Video(VideoError::DecodeError("No frame response".to_string())))?
        } else {
            Err(OtipError::Video(VideoError::InitFailed("Video not initialized".to_string())))
        }
    }

    fn hw_acceleration_available(&self) -> bool {
        // Check for hardware decoders
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
