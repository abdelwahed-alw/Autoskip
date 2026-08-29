//! Timeline management for visual seekbar

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
pub use crate::domain::{TimelineSegmentType, TimelineSegment, VideoId, ScanSegment};

/// Format duration as M:SS or H:MM:SS (short version)
pub fn format_duration_short(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{}:{:02}", minutes, seconds)
    }
}

/// Manages the visual timeline segments for a video
#[derive(Debug, Clone)]
pub struct Timeline {
    video_id: VideoId,
    duration: Duration,
    segments: Arc<RwLock<BTreeMap<u64, TimelineSegment>>>, // key = start_time in millis
    scan_coverage: Arc<RwLock<f32>>, // 0.0 to 1.0
}

impl Timeline {
    pub fn new(video_id: VideoId, duration: Duration) -> Self {
        Self {
            video_id,
            duration,
            segments: Arc::new(RwLock::new(BTreeMap::new())),
            scan_coverage: Arc::new(RwLock::new(0.0)),
        }
    }

    /// Add or update a segment
    pub fn add_segment(&self, segment: TimelineSegment) {
        let key = segment.start_time.as_millis() as u64;
        self.segments.write().insert(key, segment);
        self.update_coverage();
    }

    /// Add multiple segments
    pub fn add_segments(&self, segments: Vec<TimelineSegment>) {
        let mut map = self.segments.write();
        for segment in segments {
            let key = segment.start_time.as_millis() as u64;
            map.insert(key, segment);
        }
        drop(map);
        self.update_coverage();
    }

    /// Get segment at a specific time
    pub fn get_segment_at(&self, time: Duration) -> Option<TimelineSegment> {
        let key = time.as_millis() as u64;
        let segments = self.segments.read();
        
        // Find the segment that contains this time
        segments.range(..=key).next_back().map(|(_, v)| v.clone())
    }

    /// Get all segments in a time range
    pub fn get_segments_in_range(&self, start: Duration, end: Duration) -> Vec<TimelineSegment> {
        let start_key = start.as_millis() as u64;
        let end_key = end.as_millis() as u64;
        let segments = self.segments.read();
        
        segments
            .range(start_key..=end_key)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// Get all segments
    pub fn get_all_segments(&self) -> Vec<TimelineSegment> {
        self.segments.read().values().cloned().collect()
    }

    /// Get explicit (NSFW) segments for auto-skip
    pub fn get_explicit_segments(&self) -> Vec<ScanSegment> {
        self.segments
            .read()
            .values()
            .filter_map(|s| {
                if s.segment_type == TimelineSegmentType::ExplicitContent {
                    s.scan_segment.clone()
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get skip zones (explicit content + small buffer)
    pub fn get_skip_zones(&self, buffer_ms: u64) -> Vec<(Duration, Duration)> {
        let explicit = self.get_explicit_segments();
        let mut zones = Vec::new();
        
        for segment in explicit {
            let start = segment.start_time.saturating_sub(Duration::from_millis(buffer_ms));
            let end = (segment.end_time + Duration::from_millis(buffer_ms)).min(self.duration);
            zones.push((start, end));
        }
        
        // Merge overlapping zones
        zones.sort_by_key(|(s, _)| *s);
        let mut merged: Vec<(Duration, Duration)> = Vec::new();
        
        for (start, end) in zones {
            if let Some((_, last_end)) = merged.last_mut() {
                if start <= *last_end {
                    *last_end = (*last_end).max(end);
                } else {
                    merged.push((start, end));
                }
            } else {
                merged.push((start, end));
            }
        }
        
        merged
    }

    /// Check if a time position is in a skip zone
    pub fn is_in_skip_zone(&self, time: Duration, buffer_ms: u64) -> bool {
        let zones = self.get_skip_zones(buffer_ms);
        zones.iter().any(|(start, end)| time >= *start && time < *end)
    }

    /// Get the next safe position after a skip zone
    pub fn get_next_safe_position(&self, time: Duration, buffer_ms: u64) -> Option<Duration> {
        let zones = self.get_skip_zones(buffer_ms);
        for (start, end) in zones {
            if time >= start && time < end {
                return Some(end);
            }
        }
        None
    }

    /// Get visual segments for rendering the timeline bar
    pub fn get_visual_segments(&self, width: u32) -> Vec<VisualSegment> {
        let segments = self.get_all_segments();
        if segments.is_empty() {
            return vec![VisualSegment {
                x: 0.0,
                width: width as f32,
                segment_type: TimelineSegmentType::Unknown,
            }];
        }

        let mut visual = Vec::new();
        let total_ms = self.duration.as_millis() as f32;
        
        for segment in segments {
            let start_x = (segment.start_time.as_millis() as f32 / total_ms) * width as f32;
            let end_x = (segment.end_time.as_millis() as f32 / total_ms) * width as f32;
            let seg_width = end_x - start_x;
            
            if seg_width > 0.5 {
                visual.push(VisualSegment {
                    x: start_x,
                    width: seg_width,
                    segment_type: segment.segment_type,
                });
            }
        }

        // Fill gaps with unknown
        visual.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        
        let mut result = Vec::new();
        let mut last_end = 0.0;
        
        for seg in visual {
            if seg.x > last_end + 0.5 {
                result.push(VisualSegment {
                    x: last_end,
                    width: seg.x - last_end,
                    segment_type: TimelineSegmentType::Unknown,
                });
            }
            result.push(seg);
            last_end = seg.x + seg.width;
        }
        
        if last_end < width as f32 - 0.5 {
            result.push(VisualSegment {
                x: last_end,
                width: width as f32 - last_end,
                segment_type: TimelineSegmentType::Unknown,
            });
        }

        result
    }

    fn update_coverage(&self) {
        let segments = self.get_all_segments();
        if segments.is_empty() {
            *self.scan_coverage.write() = 0.0;
            return;
        }

        let scanned_ms: u64 = segments
            .iter()
            .filter(|s| s.segment_type != TimelineSegmentType::Unknown)
            .map(|s| (s.end_time - s.start_time).as_millis() as u64)
            .sum();

        let total_ms = self.duration.as_millis() as u64;
        if total_ms > 0 {
            *self.scan_coverage.write() = scanned_ms as f32 / total_ms as f32;
        }
    }

    pub fn scan_coverage(&self) -> f32 {
        *self.scan_coverage.read()
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn video_id(&self) -> VideoId {
        self.video_id
    }
}

/// Visual segment for rendering
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualSegment {
    pub x: f32,
    pub width: f32,
    pub segment_type: TimelineSegmentType,
}

impl VisualSegment {
    pub fn color(&self) -> [f32; 4] {
        match self.segment_type {
            TimelineSegmentType::Unknown => [0.5, 0.5, 0.5, 1.0],      // Gray
            TimelineSegmentType::ScannedSafe => [0.0, 0.7, 0.0, 1.0],  // Green
            TimelineSegmentType::ExplicitContent => [0.9, 0.1, 0.1, 1.0], // Red
            TimelineSegmentType::SkipZone => [0.9, 0.5, 0.0, 1.0],     // Orange
        }
    }
}

/// Timeline manager for multiple videos
pub struct TimelineManager {
    timelines: Arc<RwLock<BTreeMap<VideoId, Arc<Timeline>>>>,
}

impl TimelineManager {
    pub fn new() -> Self {
        Self {
            timelines: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub fn create_timeline(&self, video_id: VideoId, duration: Duration) -> Arc<Timeline> {
        let timeline = Arc::new(Timeline::new(video_id, duration));
        self.timelines.write().insert(video_id, timeline.clone());
        timeline
    }

    pub fn get_timeline(&self, video_id: VideoId) -> Option<Arc<Timeline>> {
        self.timelines.read().get(&video_id).cloned()
    }

    pub fn remove_timeline(&self, video_id: VideoId) {
        self.timelines.write().remove(&video_id);
    }
}

impl Default for TimelineManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use crate::domain::{VideoId, ScanSegment, QuadrantFlags};

    #[test]
    fn test_timeline_basic() {
        let video_id = VideoId::new();
        let duration = Duration::from_secs(100);
        let timeline = Timeline::new(video_id, duration);

        let segment = TimelineSegment {
            start_time: Duration::from_secs(10),
            end_time: Duration::from_secs(20),
            segment_type: TimelineSegmentType::ExplicitContent,
            scan_segment: Some(ScanSegment {
                start_time: Duration::from_secs(10),
                end_time: Duration::from_secs(20),
                is_explicit: true,
                confidence: 0.9,
                quadrant_flags: Some(QuadrantFlags::new()),
            }),
        };

        timeline.add_segment(segment);
        
        let at_15s = timeline.get_segment_at(Duration::from_secs(15));
        assert!(at_15s.is_some());
        assert_eq!(at_15s.unwrap().segment_type, TimelineSegmentType::ExplicitContent);

        let at_5s = timeline.get_segment_at(Duration::from_secs(5));
        assert!(at_5s.is_none());
    }

    #[test]
    fn test_skip_zones() {
        let video_id = VideoId::new();
        let duration = Duration::from_secs(100);
        let timeline = Timeline::new(video_id, duration);

        // Add explicit segment at 10-12s
        let segment = TimelineSegment {
            start_time: Duration::from_secs(10),
            end_time: Duration::from_secs(12),
            segment_type: TimelineSegmentType::ExplicitContent,
            scan_segment: Some(ScanSegment {
                start_time: Duration::from_secs(10),
                end_time: Duration::from_secs(12),
                is_explicit: true,
                confidence: 0.9,
                quadrant_flags: None,
            }),
        };
        timeline.add_segment(segment);

        let zones = timeline.get_skip_zones(500); // 500ms buffer
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].0, Duration::from_millis(9500));
        assert_eq!(zones[0].1, Duration::from_millis(12500));

        assert!(timeline.is_in_skip_zone(Duration::from_secs(11), 500));
        assert!(!timeline.is_in_skip_zone(Duration::from_secs(8), 500));
        
        let next_safe = timeline.get_next_safe_position(Duration::from_secs(11), 500);
        assert_eq!(next_safe, Some(Duration::from_millis(12500)));
    }

    #[test]
    fn test_visual_segments() {
        let video_id = VideoId::new();
        let duration = Duration::from_secs(100);
        let timeline = Timeline::new(video_id, duration);

        // Add safe segment 0-30s
        timeline.add_segment(TimelineSegment {
            start_time: Duration::ZERO,
            end_time: Duration::from_secs(30),
            segment_type: TimelineSegmentType::ScannedSafe,
            scan_segment: None,
        });

        // Add explicit segment 30-35s
        timeline.add_segment(TimelineSegment {
            start_time: Duration::from_secs(30),
            end_time: Duration::from_secs(35),
            segment_type: TimelineSegmentType::ExplicitContent,
            scan_segment: None,
        });

        let visual = timeline.get_visual_segments(1000);
        assert_eq!(visual.len(), 3); // safe, explicit, unknown
        
        // Check colors
        assert_eq!(visual[0].segment_type, TimelineSegmentType::ScannedSafe);
        assert_eq!(visual[1].segment_type, TimelineSegmentType::ExplicitContent);
        assert_eq!(visual[2].segment_type, TimelineSegmentType::Unknown);
    }
}