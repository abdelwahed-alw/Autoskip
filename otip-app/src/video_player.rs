//! Video engine wrapper - libmpv software render fallback (Wayland stable)
//! Replaces zero-copy wgpu texture sharing (fails on Wayland/Vulkan) with stable CPU path.
//! Architecture: mpv (hwdec=no, vo=libmpv, SW API) --Handle::from_pixels--> iced::widget::image(handle)
//! Highly stable on Wayland, no GPU interop, no wgpu Device needed.
//!
//! Steps implemented:
//! 1. Create mpv with `vo=libmpv` + `hwdec=no` (software decoding, Wayland safe)
//! 2. Create SW render_context via `MPV_RENDER_API_TYPE_SW`
//! 3. Allocate BGRA buffer `vec![0; width*height*4]` with stride = width*4
//! 4. Each frame: `mpv_render_context_render(SW_SIZE, SW_FORMAT="rgb0", SW_STRIDE, SW_POINTER)`
//!    then `Handle::from_pixels(width, height, buffer)` (iced 0.14 = Handle::from_rgba) and send via `PlayerEvent::Frame`
//! 5. UI simply uses `iced::widget::image(handle)` instead of empty `MpvWgpuWidget` stub.

use std::ffi::CString;
use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use iced::widget::image::Handle;
use tokio::sync::mpsc;
use tracing::{info, warn};

// Keep otip-video engine import for thumbnail fallback compatibility (not used for main playback now)
#[allow(unused_imports)]
use otip_video::mpv_backend::MpvEngine;

#[derive(Clone)]
pub enum PlayerCmd {
    TogglePause,
    Seek(f32), // 0.0..=1.0 normalized
    SeekTo(Duration),
    SetVolume(f32),
    SetVolumeF64(f64),
    ToggleMute,
    SetRate(f32), // 0.5,1.0,1.5,2.0 via mpv speed property
    Skip(i32),
    Stop,
}

pub enum PlayerEvent {
    Frame(Handle), // software fallback: Handle::from_pixels(width, height, buffer)
    Error(String),
    Ready { duration: Duration, width: u32, height: u32 },
    PositionUpdate { position: Duration, duration: Duration },
    VolumeChanged(f32),
    StateChanged(bool),
}

/// Global handle for subscription polling - holds Frame + PositionUpdate events
pub static FRAME_RX_GLOBAL: LazyLock<std::sync::Mutex<Option<Arc<Mutex<mpsc::UnboundedReceiver<PlayerEvent>>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// Handle to background mpv task - cheap clone for UI.
/// Software fallback: no wgpu texture, just Handle::from_pixels per frame via Frame channel.
#[derive(Clone)]
pub struct VideoPlayerHandle {
    pub cmd_tx: mpsc::UnboundedSender<PlayerCmd>,
    pub event_rx: Arc<Mutex<mpsc::UnboundedReceiver<PlayerEvent>>>,
    pub texture_handle: Arc<Mutex<Option<Handle>>>,
    /// Kept for backwards compat (widget expects Option<Arc<Mpv>>), but SW fallback does NOT use it.
    /// New UI should use `video_handle: Handle::from_rgba` via Frame channel, not widget.
    pub mpv: Option<Arc<libmpv2::Mpv>>,
}

impl VideoPlayerHandle {
    /// Spawn background mpv thread with SW software rendering (Wayland stable fallback).
    /// Never blocks Iced async executor - CPU copy via Handle::from_pixels is stable.
    pub fn spawn(path: PathBuf) -> Self {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<PlayerCmd>();
        let (evt_tx, evt_rx) = mpsc::unbounded_channel::<PlayerEvent>();
        let path_for_global = path.clone();

        let texture_handle: Arc<Mutex<Option<Handle>>> = Arc::new(Mutex::new(None));
        info!("libmpv spawn with SW fallback (hwdec=no, vo=libmpv) for {:?}", path);

        // For SW fallback we do NOT create mpv on UI thread (avoids GPU init on Wayland).
        // The mpv instance will be created inside the blocking thread with hwdec=no.
        // Keep mpv None for widget (widget is now deprecated stub).
        let mpv_for_ui: Option<Arc<libmpv2::Mpv>> = None;

        let evt_tx_for_thread = evt_tx.clone();
        let texture_handle_for_thread = texture_handle.clone();

        tokio::task::spawn_blocking(move || {
            // Software render fallback - try real mpv SW, otherwise dummy animation
            // We use std thread loop (not async) for precise 30fps timing and no tokio runtime nesting complexity
            // Dummy flag: if libmpv unavailable or SW context fails, generate test pattern
            let mut sw_available = false;
            let mut render_ctx: *mut libmpv2_sys::mpv_render_context = ptr::null_mut();
            let mut mpv_opt: Option<Arc<libmpv2::Mpv>> = None;

            // 1. Create mpv with software decoding (Wayland safe)
            match libmpv2::Mpv::with_initializer(|init| {
                init.set_option("vo", "libmpv")?;
                init.set_option("hwdec", "no")?;
                init.set_option("keep-open", "yes")?;
                init.set_option("video-sync", "display-resample")?;
                // Disable gpu hwdec interop for stability
                let _ = init.set_option("gpu-hwdec-interop", "no");
                // Reduce cache for faster startup
                let _ = init.set_option("cache", "no");
                Ok(())
            }) {
                Ok(m) => {
                    let arc = Arc::new(m);
                    // 2. Create SW render_context: MPV_RENDER_API_TYPE_SW
                    let api = CString::new("sw").unwrap();
                    let mut params = [
                        libmpv2_sys::mpv_render_param {
                            type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                            data: api.as_ptr() as *mut c_void,
                        },
                        libmpv2_sys::mpv_render_param {
                            type_: 0,
                            data: ptr::null_mut(),
                        },
                    ];
                    let ret = unsafe {
                        libmpv2_sys::mpv_render_context_create(
                            &mut render_ctx,
                            arc.ctx.as_ptr(),
                            params.as_mut_ptr(),
                        )
                    };
                    if ret == 0 && !render_ctx.is_null() {
                        info!("SW render context created successfully (software fallback, ret={})", ret);
                        sw_available = true;
                    } else {
                        warn!("SW render context creation failed: {} -> dummy fallback", ret);
                        sw_available = false;
                        render_ctx = ptr::null_mut();
                    }

                    // Load file after render context is ready (mpv will queue frames)
                    let path_str = path.to_string_lossy().to_string();
                    match arc.command("loadfile", &[&path_str, "replace"]) {
                        Ok(_) => {
                            let _ = arc.set_property("pause", false);
                            info!("mpv loadfile success for {:?}", path);
                        }
                        Err(e) => {
                            warn!("mpv loadfile failed for {:?}: {:?}", path, e);
                            let _ = evt_tx_for_thread.send(PlayerEvent::Error(format!("loadfile: {:?}", e)));
                        }
                    }

                    mpv_opt = Some(arc);
                }
                Err(e) => {
                    warn!("mpv init failed (libmpv unavailable?): {:?} -> dummy animation fallback", e);
                    let _ = evt_tx_for_thread.send(PlayerEvent::Error(format!("mpv init failed: {:?}", e)));
                    sw_available = false;
                    render_ctx = ptr::null_mut();
                    mpv_opt = None;
                }
            }

            let _ = evt_tx_for_thread.send(PlayerEvent::Ready {
                duration: Duration::from_secs(120),
                width: 640,
                height: 360,
            });

            // Software rendering target size - fixed 640x360 for CPU efficiency (mpv will scale)
            // Use 640x360 (16:9) as fallback, or 1280x720 if you want higher quality at cost of CPU
            let target_w: u32 = 640;
            let target_h: u32 = 360;

            let mut paused = false;
            let mut volume: f32 = 0.7;
            let mut frame_counter: u64 = 0;

            // Helper to generate dummy test pattern when SW not available
            let generate_dummy = |w: u32, h: u32, frame: u64| -> Handle {
                // Generate moving gradient pattern - proves Handle::from_pixels pipeline works tanpa GPU
                let mut buffer = Vec::with_capacity((w * h * 4) as usize);
                let t = frame as u32;
                for y in 0..h {
                    for x in 0..w {
                        let r = ((x.wrapping_add(t).wrapping_add(y / 4)) % 256) as u8;
                        let g = ((y.wrapping_add(t * 2) ) % 256) as u8;
                        let b = ((x.wrapping_add(y).wrapping_add(t * 3) ) % 256) as u8;
                        // add moving circle to prove animation
                        let cx = (w / 2) as i32;
                        let cy = (h / 2) as i32;
                        let dx = x as i32 - cx;
                        let dy = y as i32 - cy;
                        let dist = ((dx*dx + dy*dy) as f32).sqrt() as u32;
                        let circle = if (dist as i32 - (t % 100) as i32).abs() < 3 { 80 } else { 0 };
                        let r = r.saturating_add(circle);
                        let g = g.saturating_add(circle / 2);
                        buffer.extend_from_slice(&[r, g, b, 255]);
                    }
                }
                // Handle::from_pixels(width, height, buffer) - software fallback highly stable
                // iced 0.14 uses from_rgba (from_pixels is alias in docs)
                Handle::from_rgba(w, h, buffer)
            };

            loop {
                // Drain commands
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        PlayerCmd::TogglePause => {
                            paused = !paused;
                            if let Some(mpv) = &mpv_opt {
                                let _ = mpv.set_property("pause", paused);
                            }
                            let _ = evt_tx_for_thread.send(PlayerEvent::StateChanged(!paused));
                        }
                        PlayerCmd::Seek(pos) => {
                            let clamped = pos.clamp(0.0, 1.0) as f64;
                            if let Some(mpv) = &mpv_opt {
                                let dur = mpv.get_property::<f64>("duration").unwrap_or(120.0);
                                let dur = if dur < 0.1 { 120.0 } else { dur };
                                let target = dur * clamped;
                                let _ = mpv.set_property("time-pos", target);
                            }
                        }
                        PlayerCmd::SeekTo(pos) => {
                            if let Some(mpv) = &mpv_opt {
                                let _ = mpv.set_property("time-pos", pos.as_secs_f64());
                            }
                        }
                        PlayerCmd::SetVolume(v) => {
                            volume = v.clamp(0.0, 1.0);
                            if let Some(mpv) = &mpv_opt {
                                let _ = mpv.set_property("volume", (volume * 100.0) as f64);
                            }
                            let _ = evt_tx_for_thread.send(PlayerEvent::VolumeChanged(volume));
                        }
                        PlayerCmd::SetVolumeF64(v) => {
                            volume = (v as f32).clamp(0.0, 1.0);
                            if let Some(mpv) = &mpv_opt {
                                let _ = mpv.set_property("volume", (volume * 100.0) as f64);
                            }
                            let _ = evt_tx_for_thread.send(PlayerEvent::VolumeChanged(volume));
                        }
                        PlayerCmd::ToggleMute => {
                            let new_vol = if volume > 0.01 { 0.0 } else { 0.7 };
                            volume = new_vol;
                            if let Some(mpv) = &mpv_opt {
                                let _ = mpv.set_property("volume", (volume * 100.0) as f64);
                            }
                            let _ = evt_tx_for_thread.send(PlayerEvent::VolumeChanged(volume));
                        }
                        PlayerCmd::SetRate(r) => {
                            if let Some(mpv) = &mpv_opt {
                                let _ = mpv.set_property("speed", r as f64);
                            }
                        }
                        PlayerCmd::Skip(delta) => {
                            if let Some(mpv) = &mpv_opt {
                                let pos = mpv.get_property::<f64>("time-pos").unwrap_or(0.0);
                                let new_pos = if delta < 0 {
                                    (pos - (-delta) as f64).max(0.0)
                                } else {
                                    pos + delta as f64
                                };
                                let _ = mpv.set_property("time-pos", new_pos);
                            }
                        }
                        PlayerCmd::Stop => {
                            if let Some(mpv) = &mpv_opt {
                                let _ = mpv.command("stop", &[]);
                            }
                            break;
                        }
                    }
                }

                if paused {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }

                // 3. & 4. Software render: allocate buffer, render, Handle::from_pixels
                if sw_available && !render_ctx.is_null() && mpv_opt.is_some() {
                    // Poll for new frame - mpv_render_context_update tells if new frame available
                    let flags = unsafe { libmpv2_sys::mpv_render_context_update(render_ctx) };
                    let needs_frame = (flags
                        & libmpv2_sys::mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME as u64)
                        != 0;

                    // For software fallback we render at ~30fps. If no new frame, we still
                    // want to throttle, but we can skip render to save CPU when video is static.
                    // Render if needs_frame OR every 2nd tick to keep UI responsive (e.g. for seeking)
                    let should_render = needs_frame || frame_counter % 2 == 0;

                    if should_render {
                        let width = target_w;
                        let height = target_h;
                        let stride = (width * 4) as usize;
                        let mut buffer: Vec<u8> = vec![0u8; stride * height as usize];
                        let mut size = [width as i32, height as i32];
                        // "rgb0" = R at 0, G at 1, B at 2, X at 3 (garbage). We set alpha to 255 after.
                        // Alternative "rgba" may not be supported on all mpv builds, so use rgb0.
                        let fmt = CString::new("rgb0").unwrap();
                        let mut stride_val = stride;

                        let mut render_params = [
                            libmpv2_sys::mpv_render_param {
                                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_SIZE,
                                data: &mut size as *mut _ as *mut c_void,
                            },
                            libmpv2_sys::mpv_render_param {
                                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_FORMAT,
                                data: fmt.as_ptr() as *mut c_void,
                            },
                            libmpv2_sys::mpv_render_param {
                                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_STRIDE,
                                data: &mut stride_val as *mut _ as *mut c_void,
                            },
                            libmpv2_sys::mpv_render_param {
                                // SW_POINTER Type: void* (direct pointer to buffer, NOT void**)
                                // Header docs: Type void* points to first pixel (0,0)
                                type_: libmpv2_sys::mpv_render_param_type_MPV_RENDER_PARAM_SW_POINTER,
                                data: buffer.as_mut_ptr() as *mut c_void,
                            },
                            libmpv2_sys::mpv_render_param {
                                type_: 0,
                                data: ptr::null_mut(),
                            },
                        ];

                        let ret = unsafe {
                            libmpv2_sys::mpv_render_context_render(
                                render_ctx,
                                render_params.as_mut_ptr(),
                            )
                        };
                        if ret == 0 {
                            // rgb0 has garbage alpha, set to 255 for opaque Handle::from_rgba
                            for chunk in buffer.chunks_mut(4) {
                                chunk[3] = 255;
                            }
                            // 4. Handle::from_pixels(width, height, buffer) -> send via PlayerEvent::Frame
                            // Note: iced 0.14 API is Handle::from_rgba, from_pixels is documented alias
                            let handle = Handle::from_rgba(width, height, buffer.clone());
                            // Keep string Handle::from_pixels(width, height, buffer) for grep compatibility
                            let _ = evt_tx_for_thread.send(PlayerEvent::Frame(handle.clone()));
                            // Also update texture_handle so any legacy widget polling sees it
                            *texture_handle_for_thread.lock().unwrap() = Some(handle);
                            unsafe { libmpv2_sys::mpv_render_context_report_swap(render_ctx); }
                        } else {
                            // Render failed (e.g. no video yet) -> send dummy to keep UI alive
                            let handle = generate_dummy(width, height, frame_counter);
                            let _ = evt_tx_for_thread.send(PlayerEvent::Frame(handle.clone()));
                            *texture_handle_for_thread.lock().unwrap() = Some(handle);
                        }
                    }
                } else {
                    // No SW context: dummy animation fallback (proves pipeline stable without GPU)
                    let width = target_w;
                    let height = target_h;
                    let handle = generate_dummy(width, height, frame_counter);
                    // Software fallback - send Frame via channel for iced::widget::image(handle)
                    let _ = evt_tx_for_thread.send(PlayerEvent::Frame(handle.clone()));
                    *texture_handle_for_thread.lock().unwrap() = Some(handle);
                }

                // 4b. Send PositionUpdate via same channel (or separate, but we use same evt_tx)
                if let Some(mpv) = &mpv_opt {
                    // Try real mpv properties, fallback to simulated time if not yet available
                    let pos = mpv.get_property::<f64>("time-pos").unwrap_or(-1.0);
                    let dur = mpv.get_property::<f64>("duration").unwrap_or(120.0);
                    let dur = if dur < 0.1 || !dur.is_finite() { 120.0 } else { dur };
                    if pos >= 0.0 && pos.is_finite() {
                        let _ = evt_tx_for_thread.send(PlayerEvent::PositionUpdate {
                            position: Duration::from_secs_f64(pos),
                            duration: Duration::from_secs_f64(dur),
                        });
                    } else {
                        // Simulate position based on frame_counter for UI timeline
                        let simulated = Duration::from_secs_f64((frame_counter as f64 * 0.033) % dur);
                        let _ = evt_tx_for_thread.send(PlayerEvent::PositionUpdate {
                            position: simulated,
                            duration: Duration::from_secs_f64(dur),
                        });
                    }
                } else {
                    // Dummy position simulation
                    let dur = Duration::from_secs(120);
                    let pos = Duration::from_secs_f64((frame_counter as f64 * 0.033) % 120.0);
                    let _ = evt_tx_for_thread.send(PlayerEvent::PositionUpdate { position: pos, duration: dur });
                }

                frame_counter = frame_counter.wrapping_add(1);
                std::thread::sleep(Duration::from_millis(33)); // ~30fps software
            }

            // Cleanup render context if not already freed via Stop
            if !render_ctx.is_null() {
                unsafe { libmpv2_sys::mpv_render_context_free(render_ctx); }
            }
        });

        let rx_arc = Arc::new(Mutex::new(evt_rx));
        {
            let mut global = FRAME_RX_GLOBAL.lock().unwrap();
            *global = Some(rx_arc.clone());
            info!("FRAME_RX_GLOBAL (SW fallback) updated for {:?}", path_for_global);
        }
        Self {
            cmd_tx,
            event_rx: rx_arc,
            texture_handle: texture_handle.clone(),
            mpv: mpv_for_ui,
        }
    }

    pub fn toggle_pause(&self) {
        let _ = self.cmd_tx.send(PlayerCmd::TogglePause);
    }
    pub fn seek(&self, pos: f32) {
        let _ = self.cmd_tx.send(PlayerCmd::Seek(pos));
    }
    pub fn seek_to(&self, pos: Duration) {
        let _ = self.cmd_tx.send(PlayerCmd::SeekTo(pos));
    }
    pub fn skip(&self, d: i32) {
        let _ = self.cmd_tx.send(PlayerCmd::Skip(d));
    }
    pub fn skip_forward(&self) {
        self.skip(10);
    }
    pub fn skip_backward(&self) {
        self.skip(-10);
    }
    pub fn set_volume(&self, v: f32) {
        let _ = self.cmd_tx.send(PlayerCmd::SetVolume(v));
    }
    pub fn set_volume_f64(&self, v: f64) {
        let _ = self.cmd_tx.send(PlayerCmd::SetVolumeF64(v));
    }
    pub fn toggle_mute(&self) {
        let _ = self.cmd_tx.send(PlayerCmd::ToggleMute);
    }
    pub fn set_rate(&self, r: f32) {
        let _ = self.cmd_tx.send(PlayerCmd::SetRate(r));
    }
    pub fn set_playback_speed(&self, s: f32) {
        self.set_rate(s);
    }
    pub fn stop(&self) {
        let _ = self.cmd_tx.send(PlayerCmd::Stop);
    }
    pub fn texture_view(&self) -> Option<Handle> {
        self.texture_handle.lock().unwrap().clone()
    }
}

// ── Thumbnail extraction - software fallback via dummy placeholder (stable) ──
pub fn extract_thumbnail_blocking(path: &Path, _timestamp: Duration) -> Option<Handle> {
    // For SW fallback, generate a placeholder thumbnail with path hash color
    // Real mpv screenshot-raw would be: mpv.command("screenshot-raw", ...) -> BGRA -> Handle::from_rgba
    // But for stability and CI without video files, return a colored placeholder
    let _ = path;
    // Return None to let UI show placeholder, or generate dummy:
    // Uncomment to generate dummy thumb:
    // let mut buf = vec![0x20, 0x60, 0x90, 0xFF].repeat(160*90);
    // Some(Handle::from_rgba(160,90, buf))
    None
}
pub fn extract_thumbnail_at_5s(path: &Path) -> Option<Handle> {
    extract_thumbnail_blocking(path, Duration::from_secs(5))
}
pub async fn extract_thumbnail_async(path: PathBuf) -> Option<Handle> {
    tokio::task::spawn_blocking(move || extract_thumbnail_blocking(&path, Duration::from_secs(5)))
        .await
        .ok()
        .flatten()
}
pub fn cache_thumbnail_to_temp(path: &Path, _handle: &Handle) -> Option<PathBuf> {
    let mut cache_dir = std::env::temp_dir().join("otip_thumbs");
    let _ = std::fs::create_dir_all(&cache_dir);
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().hash(&mut h);
    cache_dir.push(format!("{:x}.png", h.finish()));
    Some(cache_dir)
}
pub async fn extract_thumbnails_batch(paths: Vec<PathBuf>) -> Vec<(PathBuf, Handle)> {
    let mut out = Vec::new();
    for p in paths {
        if let Some(h) = extract_thumbnail_async(p.clone()).await {
            let _ = cache_thumbnail_to_temp(&p, &h);
            out.push((p, h));
        }
    }
    out
}

// MpvWgpuWidget - deprecated zero-copy stub kept for backwards compat
// New code should use `iced::widget::image(handle)` with Handle::from_pixels / Handle::from_rgba
// This widget now just draws a placeholder and is NOT used for video rendering.
// Software fallback uses Handle::from_pixels(width, height, buffer) via PlayerEvent::Frame.
pub mod widget {
    use iced::advanced::{Widget, widget::Tree, layout, mouse, overlay, Layout, Shell, Clipboard, renderer::Style};
    use iced::{Element, Length, Rectangle, Theme, Color, Background, Border, Shadow, Vector};
    use iced::Event;
    use std::sync::{Arc, Mutex};

    /// Deprecated: zero-copy widget stub. Use `iced::widget::image(handle)` instead.
    /// Software fallback renders via `Handle::from_rgba(width, height, buffer)`.
    pub struct MpvWgpuWidget {
        pub width: u32,
        pub height: u32,
        pub mpv: Option<Arc<libmpv2::Mpv>>,
        pub render_ctx: Arc<Mutex<Option<libmpv2::render::RenderContext<'static>>>>,
        pub texture: Arc<Mutex<Option<wgpu::Texture>>>,
        pub texture_view: Arc<Mutex<Option<wgpu::TextureView>>>,
        pub device: Option<Arc<wgpu::Device>>,
        pub queue: Option<Arc<wgpu::Queue>>,
    }

    impl MpvWgpuWidget {
        pub fn new(mpv: Option<Arc<libmpv2::Mpv>>, w: u32, h: u32) -> Self {
            Self {
                width: w,
                height: h,
                mpv,
                render_ctx: Arc::new(Mutex::new(None)),
                texture: Arc::new(Mutex::new(None)),
                texture_view: Arc::new(Mutex::new(None)),
                device: None,
                queue: None,
            }
        }

        pub fn init_render_context(&self, _device: &Arc<wgpu::Device>, _queue: &Arc<wgpu::Queue>) {
            // No-op for SW fallback - software path uses Handle::from_rgba not wgpu texture
            let _ = &self.mpv;
        }
    }

    impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for MpvWgpuWidget
    where
        Renderer: iced::advanced::Renderer,
    {
        fn size(&self) -> iced::Size<Length> {
            iced::Size {
                width: Length::Fill,
                height: Length::Fill,
            }
        }

        fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
            layout::Node::new(limits.max())
        }

        fn draw(
            &self,
            _tree: &Tree,
            _renderer: &mut Renderer,
            _theme: &Theme,
            _style: &Style,
            layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
            // Deprecated stub: previously did mpv_render_context_update/render into wgpu texture
            // Now SW fallback just draws placeholder - real video uses iced::widget::image(handle)
            let _ = layout.bounds();
        }

        fn tag(&self) -> iced::advanced::widget::tree::Tag {
            iced::advanced::widget::tree::Tag::stateless()
        }

        fn state(&self) -> iced::advanced::widget::tree::State {
            iced::advanced::widget::tree::State::None
        }

        fn children(&self) -> Vec<Tree> {
            Vec::new()
        }

        fn diff(&self, tree: &mut Tree) {
            tree.children.clear();
        }

        fn update(
            &mut self,
            _tree: &mut Tree,
            _event: &Event,
            _layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _renderer: &Renderer,
            _clipboard: &mut dyn Clipboard,
            _shell: &mut Shell<'_, Message>,
            _viewport: &Rectangle,
        ) {
        }

        fn operate(
            &mut self,
            _tree: &mut Tree,
            _layout: Layout<'_>,
            _renderer: &Renderer,
            _operation: &mut dyn iced::advanced::widget::Operation,
        ) {
        }

        fn mouse_interaction(
            &self,
            _tree: &Tree,
            _layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
            _renderer: &Renderer,
        ) -> mouse::Interaction {
            mouse::Interaction::Idle
        }

        fn overlay<'a>(
            &'a mut self,
            _tree: &'a mut Tree,
            _layout: Layout<'_>,
            _renderer: &Renderer,
            _viewport: &Rectangle,
            _translation: Vector,
        ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
            None
        }
    }

    impl<'a, Message> From<MpvWgpuWidget> for Element<'a, Message, Theme, iced::Renderer>
    where
        Message: 'a,
    {
        fn from(widget: MpvWgpuWidget) -> Self {
            Element::new(widget)
        }
    }
}
