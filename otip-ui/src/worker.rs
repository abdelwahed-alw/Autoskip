//! Background workers for async operations

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::{info, debug, warn, error};
use otip_core::domain::{VideoId, PlaybackMode, ScanProgress, TimelineSegment, VideoMetadata};
use otip_core::events::{UiCommand, BackendEvent};
use otip_core::error::Result;
use otip_core::timeline::format_duration_short;
use otip_video::engine::{VideoEngine, VideoEngineFactory, EngineType};
use otip_video::frame_extractor::GridFrameExtractor;
use otip_ai::moderator::{ContentModerator, ModerationConfig};

/// Worker handle for managing background tasks
pub struct WorkerHandle {
    pub(crate) command_tx: mpsc::UnboundedSender<UiCommand>,
    pub(crate) event_tx: mpsc::UnboundedSender<BackendEvent>,
    pub(crate) video_engine: Option<Arc<tokio::sync::Mutex<Box<dyn VideoEngine>>>>,
    pub(crate) content_moderator: Option<Arc<ContentModerator>>,
    scan_task: Option<JoinHandle<()>>,
    event_task: Option<JoinHandle<()>>,
    current_video: Option<VideoId>,
}

impl WorkerHandle {
    pub fn new(
        command_tx: mpsc::UnboundedSender<UiCommand>,
        event_tx: mpsc::UnboundedSender<BackendEvent>,
    ) -> Self {
        Self {
            command_tx,
            event_tx,
            video_engine: None,
            content_moderator: None,
            scan_task: None,
            event_task: None,
            current_video: None,
        }
    }

    pub fn set_video_engine(&mut self, engine: Box<dyn VideoEngine>) {
        self.video_engine = Some(Arc::new(tokio::sync::Mutex::new(engine)));
    }

    pub fn set_content_moderator(&mut self, moderator: ContentModerator) {
        self.content_moderator = Some(Arc::new(moderator));
    }

    /// Handle UI commands
    pub async fn handle_command(&mut self, command: UiCommand) -> Result<()> {
        match command {
            UiCommand::OpenVideo { path, mode } => {
                self.open_video(path, mode).await?;
            }
            UiCommand::Play(video_id) => {
                if let Some(engine) = &self.video_engine {
                    engine.lock().await.play(video_id).await?;
                }
            }
            UiCommand::Pause(video_id) => {
                if let Some(engine) = &self.video_engine {
                    engine.lock().await.pause(video_id).await?;
                }
            }
            UiCommand::Stop(video_id) => {
                if let Some(engine) = &self.video_engine {
                    engine.lock().await.stop(video_id).await?;
                }
            }
            UiCommand::Seek(video_id, position) => {
                if let Some(engine) = &self.video_engine {
                    engine.lock().await.seek(video_id, position).await?;
                }
            }
            UiCommand::SetVolume(video_id, volume) => {
                if let Some(engine) = &self.video_engine {
                    engine.lock().await.set_volume(video_id, volume).await?;
                }
            }
            UiCommand::SetPlaybackRate(video_id, rate) => {
                if let Some(engine) = &self.video_engine {
                    engine.lock().await.set_rate(video_id, rate).await?;
                }
            }
            UiCommand::ToggleFullscreen(video_id) => {
                // Handled in UI
                let _ = video_id;
            }
            UiCommand::RequestScan(video_id) => {
                self.start_scan(video_id).await?;
            }
            UiCommand::CancelScan(video_id) => {
                self.cancel_scan(video_id).await?;
            }
            UiCommand::SkipSegment(video_id, from, to) => {
                self.handle_auto_skip(video_id, from, to).await?;
            }
            UiCommand::UpdatePreferences(prefs) => {
                // Update moderator config
                if let Some(moderator) = &self.content_moderator {
                    // Would update config here
                    let _ = prefs;
                }
            }
            UiCommand::Shutdown => {
                self.shutdown().await?;
            }
        }
        Ok(())
    }

    async fn open_video(&mut self, path: String, mode: PlaybackMode) -> Result<()> {
        let video_id = VideoId::new();
        self.current_video = Some(video_id);

        // Initialize video engine
        if let Some(engine) = &self.video_engine {
            let metadata = engine.lock().await.initialize(video_id, &path).await?;
            
            // Create timeline
            let timeline = Arc::new(otip_core::timeline::Timeline::new(video_id, metadata.duration));
            
            // Send video opened event
            let _ = self.event_tx.send(BackendEvent::VideoOpened { video_id, metadata: metadata.clone() });
            
            // Start playback based on mode
            match mode {
                PlaybackMode::SafeMode => {
                    // Start scan first, then play
                    self.start_scan(video_id).await?;
                    // Playback will start after scan completes (handled by scan completion)
                }
                PlaybackMode::InstantPlay => {
                    // Start playback immediately
                    engine.lock().await.play(video_id).await?;
                    // Start scan in background
                    self.start_scan(video_id).await?;
                }
            }
        }

        Ok(())
    }

    async fn start_scan(&mut self, video_id: VideoId) -> Result<()> {
        if let (Some(engine), Some(moderator)) = (&self.video_engine, &self.content_moderator) {
            // Get video duration
            let (_, duration) = engine.lock().await.get_position(video_id).await?;
            
            // Create frame extraction channel
            let (frame_tx, frame_rx) = mpsc::unbounded_channel();
            
            // Start frame extractor
            let engine_clone = engine.clone();
            let extractor = GridFrameExtractor::new(
                engine_clone,
                Duration::from_secs(1),
                (320, 240),
                (2, 2),
            );
            
            let extractor_task = tokio::spawn(async move {
                if let Err(e) = extractor.extract_grids(video_id, duration, frame_tx).await {
                    error!("Frame extraction failed: {}", e);
                }
            });

            // Start moderation
            let moderator_clone = moderator.clone();
            let event_tx = self.event_tx.clone();
            
            self.scan_task = Some(tokio::spawn(async move {
                if let Err(e) = moderator_clone.moderate_video(video_id, String::new(), duration, frame_rx).await {
                    error!("Moderation failed: {}", e);
                    let _ = event_tx.send(BackendEvent::ScanError { video_id, error: e.to_string() });
                }
            }));

            // Wait for extractor
            let _ = extractor_task.await;
        }
        Ok(())
    }

    async fn cancel_scan(&mut self, video_id: VideoId) -> Result<()> {
        if let Some(moderator) = &self.content_moderator {
            moderator.cancel_scan(video_id).await;
        }
        
        if let Some(task) = self.scan_task.take() {
            task.abort();
        }
        
        Ok(())
    }

    async fn handle_auto_skip(&mut self, video_id: VideoId, from: Duration, to: Duration) -> Result<()> {
        if let Some(engine) = &self.video_engine {
            info!("Auto-skip: {} -> {}", 
                format_duration_short(from),
                format_duration_short(to)
            );
            
            // Seek to skip position
            engine.lock().await.seek(video_id, to).await?;
            
            // Send event
            let _ = self.event_tx.send(BackendEvent::AutoSkipTriggered { video_id, from, to });
        }
        Ok(())
    }

    /// Process backend events and forward to UI
    pub async fn process_events(&mut self, mut event_rx: mpsc::UnboundedReceiver<BackendEvent>) {
        while let Some(event) = event_rx.recv().await {
            // Events are sent directly to UI via the event_tx channel
            // This method would be used if we need to process events before forwarding
            let _ = event;
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        if let Some(video_id) = self.current_video {
            if let Some(engine) = &self.video_engine {
                engine.lock().await.shutdown(video_id).await?;
            }
        }
        
        if let Some(task) = self.scan_task.take() {
            task.abort();
        }
        
        if let Some(task) = self.event_task.take() {
            task.abort();
        }
        
        Ok(())
    }
}

/// Create the video engine based on features
pub fn create_video_engine() -> Box<dyn VideoEngine> {
    VideoEngineFactory::create_best()
}

/// Create content moderator with default config
pub fn create_content_moderator(
    progress_tx: mpsc::UnboundedSender<ScanProgress>,
    segment_tx: mpsc::UnboundedSender<(VideoId, TimelineSegment)>,
) -> Result<ContentModerator> {
    let config = ModerationConfig::default();
    ContentModerator::new(config, progress_tx, segment_tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn test_worker_creation() {
        let (cmd_tx, _) = mpsc::unbounded_channel();
        let (evt_tx, _) = mpsc::unbounded_channel();
        
        let worker = WorkerHandle::new(cmd_tx, evt_tx);
        assert!(worker.video_engine.is_none());
        assert!(worker.content_moderator.is_none());
    }
}