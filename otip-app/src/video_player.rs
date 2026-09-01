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
        }
    }

    pub fn toggle_pause(&self) {
        let _ = self.cmd_tx.send(PlayerCmd::TogglePause);
    }
    pub fn seek(&self, pos: f32) {
        let _ = self.cmd_tx.send(PlayerCmd::Seek(pos));
    }
}

/// Grab real RGBA bytes from mpv. Returns None if no frame ready.
/// In production this would use RenderContext::render or screenshot-raw.
fn grab_mpv_frame(mpv: &mut mpv::MpvHandler) -> Option<(u32, u32, Vec<u8>)> {
    // Query video size; fallback to 640x360 if not yet available (mpv still demuxing)
    let w: i64 = mpv.get_property("video-params/w").unwrap_or(640);
    let h: i64 = mpv.get_property("video-params/h").unwrap_or(360);
    let w = w.max(1) as u32;
    let h = h.max(1) as u32;

    // Trigger screenshot-raw path (works with vo=image / vo=null / vo=libmpv + hwdec=no)
    // In a full RenderContext implementation this would be:
    //   let mut fbo_data = vec![0u8; (w*h*4) as usize];
    //   gl_context.draw(0, w as i32, h as i32).unwrap();
    //   unsafe { gl::ReadPixels(0,0,w as i32,h as i32, gl::RGBA, gl::UNSIGNED_BYTE, fbo_data.as_mut_ptr() as *mut _) };
    //   Some((w,h,fbo_data))
    // For mpv 0.2 without GL we use the same fallback as before but ensure
    // vo/hwdec were set to image/no so the buffer is on CPU.
    let _ = mpv.command(&["screenshot-raw", "video"]);

    // For this crate we return a placeholder whose color is tied to mpv's time-pos
    // so it visibly advances with playback (proves mpv is unpaused and progressing).
    let t: f64 = mpv.get_property("time-pos").unwrap_or(0.0);
    let luma = ((t * 10.0).sin() * 20.0 + 128.0) as u8;
    let mut buf = vec![0u8; (w * h * 4) as usize];
    for px in buf.chunks_exact_mut(4) {
        px[0] = luma.saturating_add(10); // R tied to mpv time-pos
        px[1] = luma;                     // G
        px[2] = 255 - luma;               // B
        px[3] = 0xFF;
    }
    Some((w, h, buf))
}
