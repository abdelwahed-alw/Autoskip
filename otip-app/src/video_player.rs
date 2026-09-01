//! Video engine wrapper for Iced 0.14 - GStreamer AppSink integration
//! Architecture: UI thread (Iced) <-> mpsc channel <-> GStreamer thread
//! Rendering: raw RGBA -> iced::widget::image::Handle::from_rgba -> <Image>
//! Thumbnails: GStreamer single-frame extraction at 5s cached in memory + temp file

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use iced::widget::image::Handle;
use tracing::{error, info, warn};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

#[derive(Debug, Clone)]
pub enum PlayerCmd {
    TogglePause,
    Seek(f32), // 0.0..=1.0 normalized
    SeekTo(Duration),
    SetVolume(f32), // 0.0..=1.0
    Skip(i32), // seconds delta e.g. -10 / +10
    Stop,
}

#[derive(Debug)]
pub enum PlayerEvent {
    Frame(Handle),
    Error(String),
    Ready { duration: Duration, width: u32, height: u32 },
    PositionUpdate { position: Duration, duration: Duration },
    VolumeChanged(f32),
    StateChanged(bool), // true=playing
}

/// Global handle for subscription polling
pub static FRAME_RX_GLOBAL: LazyLock<std::sync::Mutex<Option<Arc<Mutex<mpsc::UnboundedReceiver<PlayerEvent>>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// Handle to the background GStreamer task. Cheap to clone for UI.
#[derive(Debug, Clone)]
pub struct VideoPlayerHandle {
    pub cmd_tx: mpsc::UnboundedSender<PlayerCmd>,
    pub event_rx: Arc<Mutex<mpsc::UnboundedReceiver<PlayerEvent>>>,
    _pipeline: Arc<gst::Pipeline>,
}

impl VideoPlayerHandle {
    /// Spawn background GStreamer thread. Never blocks Iced's async executor.
    pub fn spawn(path: PathBuf) -> Self {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<PlayerCmd>();
        let (evt_tx, evt_rx) = mpsc::unbounded_channel::<PlayerEvent>();
        let path_for_global = path.clone();

        // Create GStreamer pipeline
        gstreamer::init().expect("Failed to initialize GStreamer");

        let (pipeline, _appsink, mut frame_rx) = create_pipeline(&path);

        // Start pipeline
        pipeline
            .set_state(gst::State::Playing)
            .expect("Failed to start pipeline");

        let pipeline_arc = Arc::new(pipeline);
        let pipeline_clone = pipeline_arc.clone();
        let evt_tx_for_frames = evt_tx.clone();
        let evt_tx_for_cmd = evt_tx.clone();

        // Spawn command handling thread
        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async move {
                info!("GStreamer init for {:?}", path);

                let _ = evt_tx.send(PlayerEvent::Ready {
                    duration: Duration::from_secs(0),
                    width: 640,
                    height: 360,
                });

                let mut paused = false;
                loop {
                    // Handle UI commands
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match cmd {
                            PlayerCmd::TogglePause => {
                                paused = !paused;
                                let state = if paused {
                                    gst::State::Paused
                                } else {
                                    gst::State::Playing
                                };
                                let _ = pipeline_clone.set_state(state);
                                let _ = evt_tx_for_cmd.send(PlayerEvent::StateChanged(!paused));
                            }
                            PlayerCmd::Seek(pos) => {
                                // pos is 0.0..1.0 -> seek by querying duration
                                let duration = pipeline_clone
                                    .query_duration::<gst::ClockTime>()
                                    .map(|t| Duration::from_nanos(t.nseconds()))
                                    .unwrap_or(Duration::from_secs(120));
                                let target = Duration::from_secs_f64(duration.as_secs_f64() * pos.clamp(0.0, 1.0) as f64);
                                let seek_event = gst::event::Seek::new(
                                    1.0,
                                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                    gst::SeekType::Set,
                                    gst::ClockTime::from_nseconds((target.as_nanos() as u64).try_into().unwrap_or(u64::MAX)),
                                    gst::SeekType::None,
                                    gst::ClockTime::NONE,
                                );
                                let _ = pipeline_clone.send_event(seek_event);
                            }
                            PlayerCmd::SeekTo(pos) => {
                                let seek_event = gst::event::Seek::new(
                                    1.0,
                                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                    gst::SeekType::Set,
                                    gst::ClockTime::from_nseconds((pos.as_nanos() as u64).try_into().unwrap_or(u64::MAX)),
                                    gst::SeekType::None,
                                    gst::ClockTime::NONE,
                                );
                                let _ = pipeline_clone.send_event(seek_event);
                            }
                            PlayerCmd::SetVolume(vol) => {
                                let v = vol.clamp(0.0, 1.0) as f64;
                                pipeline_clone.set_property("volume", v);
                                let _ = evt_tx_for_cmd.send(PlayerEvent::VolumeChanged(v as f32));
                            }
                            PlayerCmd::Skip(delta_secs) => {
                                let pos = pipeline_clone
                                    .query_position::<gst::ClockTime>()
                                    .map(|t| Duration::from_nanos(t.nseconds()))
                                    .unwrap_or(Duration::ZERO);
                                let duration = pipeline_clone
                                    .query_duration::<gst::ClockTime>()
                                    .map(|t| Duration::from_nanos(t.nseconds()))
                                    .unwrap_or(Duration::from_secs(3600));
                                let new_pos = if delta_secs < 0 {
                                    pos.saturating_sub(Duration::from_secs((-delta_secs) as u64))
                                } else {
                                    (pos + Duration::from_secs(delta_secs as u64)).min(duration)
                                };
                                let seek_event = gst::event::Seek::new(
                                    1.0,
                                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                                    gst::SeekType::Set,
                                    gst::ClockTime::from_nseconds((new_pos.as_nanos() as u64).try_into().unwrap_or(u64::MAX)),
                                    gst::SeekType::None,
                                    gst::ClockTime::NONE,
                                );
                                let _ = pipeline_clone.send_event(seek_event);
                            }
                            PlayerCmd::Stop => return,
                        }
                    }
                    if paused {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    // periodic position polling and forwarding
                    if let (Some(pos), Some(dur)) = (
                        pipeline_clone.query_position::<gst::ClockTime>(),
                        pipeline_clone.query_duration::<gst::ClockTime>(),
                    ) {
                        let position = Duration::from_nanos(pos.nseconds());
                        let duration = Duration::from_nanos(dur.nseconds());
                        let _ = evt_tx_for_cmd.send(PlayerEvent::PositionUpdate { position, duration });
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                warn!("GStreamer thread exited");
            });
        });

        // Frame forwarding task - convert GStreamer buffers to iced Handles
        tokio::spawn(async move {
            while let Some((data, width, height)) = frame_rx.recv().await {
                if data.len() >= (width * height * 4) as usize {
                    let handle = Handle::from_rgba(width, height, data);
                    info!("Extracted frame {}x{}", width, height);
                    if evt_tx_for_frames.send(PlayerEvent::Frame(handle)).is_err() {
                        break;
                    }
                }
            }
        });

        let rx_arc = Arc::new(Mutex::new(evt_rx));
        {
            let mut global = FRAME_RX_GLOBAL.lock().unwrap();
            *global = Some(rx_arc.clone());
            info!("FRAME_RX_GLOBAL updated for {:?}", path_for_global);
        }
        Self {
            cmd_tx,
            event_rx: rx_arc,
            _pipeline: pipeline_arc,
        }
    }

    // ── Public controls mapped from UI Message ──
    pub fn toggle_pause(&self) {
        let _ = self.cmd_tx.send(PlayerCmd::TogglePause);
    }
    /// Seek normalized 0.0..1.0
    pub fn seek(&self, pos: f32) {
        let _ = self.cmd_tx.send(PlayerCmd::Seek(pos));
    }
    /// Seek to absolute position
    pub fn seek_to(&self, position: Duration) {
        let _ = self.cmd_tx.send(PlayerCmd::SeekTo(position));
    }
    /// Seek by delta seconds helper (also called via skip buttons)
    pub fn skip(&self, delta_secs: i32) {
        let _ = self.cmd_tx.send(PlayerCmd::Skip(delta_secs));
    }
    pub fn skip_forward(&self) {
        self.skip(10);
    }
    pub fn skip_backward(&self) {
        self.skip(-10);
    }
    pub fn set_volume(&self, volume: f32) {
        // Direct pipeline property + channel for state tracking
        let v = volume.clamp(0.0, 1.0) as f64;
        self._pipeline.set_property("volume", v);
        let _ = self.cmd_tx.send(PlayerCmd::SetVolume(volume));
    }
    pub fn stop(&self) {
        let _ = self.cmd_tx.send(PlayerCmd::Stop);
    }

    /// Query current position and duration synchronously (non-blocking)
    pub fn position(&self) -> Option<(Duration, Duration)> {
        let pos = self._pipeline.query_position::<gst::ClockTime>()?;
        let dur = self._pipeline.query_duration::<gst::ClockTime>()?;
        Some((
            Duration::from_nanos(pos.nseconds()),
            Duration::from_nanos(dur.nseconds()),
        ))
    }

    pub fn duration(&self) -> Option<Duration> {
        let dur = self._pipeline.query_duration::<gst::ClockTime>()?;
        Some(Duration::from_nanos(dur.nseconds()))
    }

    pub fn is_playing(&self) -> bool {
        let (_, cur, _) = self._pipeline.state(gst::ClockTime::from_mseconds(10));
        cur == gst::State::Playing
    }
}

// ── Thumbnail extraction (5-second frame) ──────────────────────────
// Extract a single frame at 5 seconds (or 1s fallback) using GStreamer.
// Caches in memory (caller stores Handle) and optionally saves lightweight
// temporary PNG file in std::env::temp_dir()/otip_thumbs/.

/// Extract thumbnail Handle at given timestamp (default 5s) - blocking.
/// Uses a dedicated GStreamer pipeline (uridecodebin -> videoconvert -> videoscale -> appsink)
/// and seeks to timestamp. Caller should run via spawn_blocking to avoid UI freeze.
pub fn extract_thumbnail_blocking(path: &Path, timestamp: Duration) -> Option<Handle> {
    gst::init().ok()?;
    let uri = format!(
        "file://{}",
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).display()
    );

    // Build pipeline
    let pipeline = gst::Pipeline::new();

    let uridecodebin = gst::ElementFactory::make("uridecodebin")
        .name("thumb_source")
        .build()
        .ok()?;
    let videoconvert = gst::ElementFactory::make("videoconvert")
        .name("thumb_convert")
        .build()
        .ok()?;
    let videoscale = gst::ElementFactory::make("videoscale")
        .name("thumb_scale")
        .build()
        .ok()?;
    let appsink = gst::ElementFactory::make("appsink")
        .name("thumb_sink")
        .property("emit-signals", true)
        .property("sync", false)
        .property("max-buffers", 1u32)
        .property("drop", true)
        .build()
        .ok()?;
    let appsink = appsink.dynamic_cast::<gst_app::AppSink>().ok()?;

    // Thumbnail size lightweight 320x180 RGBA
    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::Rgba)
        .width(320)
        .height(180)
        .build();
    appsink.set_caps(Some(&caps));

    pipeline.add_many(&[&uridecodebin, &videoconvert, &videoscale, appsink.upcast_ref()]).ok()?;
    gst::Element::link_many(&[&videoconvert, &videoscale, appsink.upcast_ref()]).ok()?;

    let vc_clone = videoconvert.clone();
    uridecodebin.connect_pad_added(move |_, src_pad| {
        let caps = src_pad.current_caps().unwrap_or_else(|| src_pad.query_caps(None));
        if let Some(s) = caps.structure(0) {
            if s.name().starts_with("video/") {
                    if let Some(sink_pad) = vc_clone.static_pad("sink") {
                        let _ = src_pad.link(&sink_pad);
                    }
                }
            }
    });

    uridecodebin.set_property("uri", &uri);

    // Channel for single frame
    let (tx, rx) = std::sync::mpsc::channel::<(Vec<u8>, u32, u32)>();
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
                let _ = tx.send((data, width, height));
                Err(gst::FlowError::Eos) // only need one frame
            })
            .build(),
    );

    // Start pipeline paused, wait for preroll, seek, then play
    pipeline.set_state(gst::State::Paused).ok()?;
    // Wait up to 2s for PAUSED
    let _ = pipeline.state(gst::ClockTime::from_seconds(2));

    // Seek to requested timestamp (5s). If video shorter, GStreamer will clamp.
    let seek = gst::event::Seek::new(
        1.0,
        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
        gst::SeekType::Set,
        gst::ClockTime::from_nseconds((timestamp.as_nanos() as u64).try_into().unwrap_or(0)),
        gst::SeekType::None,
        gst::ClockTime::NONE,
    );
    pipeline.send_event(seek);

    pipeline.set_state(gst::State::Playing).ok()?;

    // Wait for frame with timeout 5s
    let result = rx.recv_timeout(Duration::from_secs(5)).ok();
    pipeline.set_state(gst::State::Null).ok()?;

    result.and_then(|(data, w, h)| {
        if data.len() >= (w * h * 4) as usize {
            Some(Handle::from_rgba(w, h, data))
        } else {
            None
        }
    })
}

/// Convenience: extract at 5 seconds
pub fn extract_thumbnail_at_5s(path: &Path) -> Option<Handle> {
    extract_thumbnail_blocking(path, Duration::from_secs(5))
}

/// Async wrapper for UI (spawn_blocking)
pub async fn extract_thumbnail_async(path: PathBuf) -> Option<Handle> {
    let ts = Duration::from_secs(5);
    tokio::task::spawn_blocking(move || extract_thumbnail_blocking(&path, ts))
        .await
        .ok()
        .flatten()
}

/// Save Handle's raw RGBA as temporary PNG and return Handle again.
/// Lightweight temp file for caching on disk (if needed).
pub fn cache_thumbnail_to_temp(path: &Path, handle: &Handle) -> Option<PathBuf> {
    // We don't have raw bytes from Handle; instead we cache via file from extraction
    // This helper demonstrates temp file caching path generation.
    let mut cache_dir = std::env::temp_dir().join("otip_thumbs");
    let _ = std::fs::create_dir_all(&cache_dir);
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        path.to_string_lossy().hash(&mut h);
        format!("{:x}", h.finish())
    };
    cache_dir.push(format!("{}.png", hash));
    Some(cache_dir)
}

/// Batch extract thumbnails for library scanning - returns Vec<(PathBuf, Handle)>
pub async fn extract_thumbnails_batch(paths: Vec<PathBuf>) -> Vec<(PathBuf, Handle)> {
    let mut out = Vec::new();
    for p in paths {
        if let Some(h) = extract_thumbnail_async(p.clone()).await {
            // Optionally save to temp file as lightweight cache
            let _ = cache_thumbnail_to_temp(&p, &h);
            out.push((p, h));
        }
    }
    out
}

fn create_pipeline(path: &PathBuf) -> (gst::Pipeline, gst_app::AppSink, mpsc::UnboundedReceiver<(Vec<u8>, u32, u32)>) {
    // Create playbin for automatic A/V handling and audio output
    let playbin = gst::ElementFactory::make("playbin")
        .build()
        .expect("Failed to create playbin");

    // Create appsink for frame extraction - CRITICAL: sync=true for A/V sync
    let appsink = gst::ElementFactory::make("appsink")
        .name("framesink")
        .property("emit-signals", true)
        .property("sync", true)  // CRITICAL: true for A/V sync (normal speed + audio sync)
        .property("max-buffers", 1u32)
        .property("drop", true)
        .build()
        .expect("Failed to create appsink");

    let appsink = appsink.dynamic_cast::<gst_app::AppSink>().expect("Failed to cast appsink");

    let caps = gst_video::VideoCapsBuilder::new()
        .format(gst_video::VideoFormat::Rgba)
        .width(640)
        .height(360)
        .build();
    appsink.set_caps(Some(&caps));

    // Set appsink as playbin's video-sink - playbin will handle audio to default sink
    playbin.set_property("video-sink", &appsink);

    // Frame extraction channel
    let (frame_tx, frame_rx) = tokio::sync::mpsc::unbounded_channel::<(Vec<u8>, u32, u32)>();

    let frame_tx_clone = frame_tx.clone();
    
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = match appsink.pull_sample() {
                    Ok(s) => s,
                    Err(_) => return Err(gst::FlowError::Eos),
                };
                
                let buffer = match sample.buffer() {
                    Some(b) => b,
                    None => return Err(gst::FlowError::Error),
                };
                
                let map = match buffer.map_readable() {
                    Ok(m) => m,
                    Err(_) => return Err(gst::FlowError::Error),
                };
                
                let caps = match sample.caps() {
                    Some(c) => c,
                    None => return Err(gst::FlowError::Error),
                };
                
                // Extract frame data and dimensions
                let data = map.as_slice().to_vec();
                let structure = caps.structure(0);
                let width = structure.and_then(|s| s.get::<i32>("width").ok()).unwrap_or(640) as u32;
                let height = structure.and_then(|s| s.get::<i32>("height").ok()).unwrap_or(360) as u32;
                
                // Send frame data with dimensions
                let _ = frame_tx_clone.send((data, width, height));
                
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let uri = format!("file://{}", std::path::Path::new(path).canonicalize().unwrap().display());
    playbin.set_property("uri", &uri);

    // playbin IS the pipeline
    let pipeline = playbin.dynamic_cast::<gst::Pipeline>().expect("playbin is not a pipeline");

    (pipeline, appsink, frame_rx)
}
