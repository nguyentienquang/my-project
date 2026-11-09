use crate::config::SurveillanceConfig;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const ANALYSIS_WIDTH: usize = 160;
pub const ANALYSIS_HEIGHT: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionEvent {
    pub id: String,
    pub timestamp: u64, // Epoch milliseconds
    pub motion_score: f32,
    pub camera_id: String,
    pub snapshot_path: String,
    pub event_type: String,
    pub region: Option<[f32; 4]>,
    #[serde(default)]
    pub video_path: Option<String>,
    #[serde(default)]
    pub has_video: bool,
    #[serde(default = "default_entity_type")]
    pub entity_type: String,
    #[serde(default)]
    pub confidence: f32,
}

fn default_entity_type() -> String {
    "motion".to_string()
}

#[derive(Debug, Clone)]
pub struct MotionResult {
    pub is_alert_triggered: bool,
    pub motion_score: f32,
    pub consecutive_frames: u32,
    pub entity_type: String,
    pub confidence: f32,
    pub bounding_box: Option<[f32; 4]>,
}

pub struct MotionDetector {
    previous_buffer: Vec<u8>,
    current_buffer: Vec<u8>,
    has_reference_frame: bool,
    consecutive_detections: u32,
    last_alert_time: Option<Instant>,
    last_analysis_time: Option<Instant>,
    smoothed_bbox: Option<[f32; 4]>,
    smoothed_confidence: f32,
    tracking_miss_count: u32,
    stable_entity: String,
    person_hold_frames: u32,
    had_active_detection: bool,
    last_rotation: Option<i32>,
}

impl Default for MotionDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionDetector {
    pub fn new() -> Self {
        Self {
            previous_buffer: vec![0u8; ANALYSIS_WIDTH * ANALYSIS_HEIGHT],
            current_buffer: vec![0u8; ANALYSIS_WIDTH * ANALYSIS_HEIGHT],
            has_reference_frame: false,
            consecutive_detections: 0,
            last_alert_time: None,
            last_analysis_time: None,
            smoothed_bbox: None,
            smoothed_confidence: 0.0,
            tracking_miss_count: 0,
            stable_entity: "motion".to_string(),
            person_hold_frames: 0,
            had_active_detection: false,
            last_rotation: None,
        }
    }

    pub fn reset(&mut self) {
        self.has_reference_frame = false;
        self.consecutive_detections = 0;
        self.last_alert_time = None;
        self.last_analysis_time = None;
        self.smoothed_bbox = None;
        self.smoothed_confidence = 0.0;
        self.tracking_miss_count = 0;
        self.stable_entity = "motion".to_string();
        self.person_hold_frames = 0;
        self.had_active_detection = false;
        self.last_rotation = None;
    }

    pub fn update_active_detection_state(&mut self, is_active: bool) -> bool {
        if is_active {
            self.had_active_detection = true;
            false
        } else if self.had_active_detection {
            self.had_active_detection = false;
            true
        } else {
            false
        }
    }

    /// Directly samples and rotates the full-resolution Y plane into the ANALYSIS_WIDTH x ANALYSIS_HEIGHT buffer.
    /// Zero allocations, zero full-frame copying.
    pub fn downsample_and_rotate_y(
        src_y: &[u8],
        src_w: usize,
        src_h: usize,
        row_stride: usize,
        rotation_degrees: i32,
        dst: &mut [u8],
    ) {
        if src_y.is_empty() || src_w == 0 || src_h == 0 || dst.len() < ANALYSIS_WIDTH * ANALYSIS_HEIGHT {
            return;
        }

        let norm_rot = ((rotation_degrees % 360) + 360) % 360;
        let y_len = src_y.len();

        let (oriented_w, oriented_h) = match norm_rot {
            90 | 270 => (src_h, src_w),
            _ => (src_w, src_h),
        };

        for dy in 0..ANALYSIS_HEIGHT {
            let out_y = (dy * oriented_h) / ANALYSIS_HEIGHT;
            let dst_row = dy * ANALYSIS_WIDTH;

            for dx in 0..ANALYSIS_WIDTH {
                let out_x = (dx * oriented_w) / ANALYSIS_WIDTH;

                let (orig_x, orig_y) = match norm_rot {
                    90 => (out_y, src_h.saturating_sub(1).saturating_sub(out_x)),
                    180 => (
                        src_w.saturating_sub(1).saturating_sub(out_x),
                        src_h.saturating_sub(1).saturating_sub(out_y),
                    ),
                    270 => (src_w.saturating_sub(1).saturating_sub(out_y), out_x),
                    _ => (out_x, out_y),
                };

                let y_idx = orig_y * row_stride + orig_x;
                dst[dst_row + dx] = if y_idx < y_len { src_y[y_idx] } else { 0 };
            }
        }
    }

    /// Downsamples a full-resolution Y-plane into the fixed ANALYSIS_WIDTH x ANALYSIS_HEIGHT buffer.
    pub fn downsample_y_plane(
        src: &[u8],
        src_w: usize,
        src_h: usize,
        row_stride: usize,
        dst: &mut [u8],
    ) {
        if src.is_empty() || src_w == 0 || src_h == 0 {
            return;
        }

        let step_x = src_w / ANALYSIS_WIDTH;
        let step_y = src_h / ANALYSIS_HEIGHT;

        if step_x == 0 || step_y == 0 {
            // Source is smaller than analysis resolution: direct copy/fallback
            let len = dst.len().min(src.len());
            dst[..len].copy_from_slice(&src[..len]);
            return;
        }

        for y in 0..ANALYSIS_HEIGHT {
            let src_y = y * step_y;
            let src_row_start = src_y * row_stride;
            let dst_row_start = y * ANALYSIS_WIDTH;

            for x in 0..ANALYSIS_WIDTH {
                let src_x = x * step_x;
                let src_idx = src_row_start + src_x;
                if src_idx < src.len() {
                    dst[dst_row_start + x] = src[src_idx];
                }
            }
        }
    }

    /// Analyzes raw unrotated Y frame directly, downscaling and rotating on the fly with zero heap allocations.
    pub fn process_raw_frame(
        &mut self,
        src_y: &[u8],
        src_w: usize,
        src_h: usize,
        row_stride: usize,
        rotation_degrees: i32,
        config: &SurveillanceConfig,
        now: Instant,
    ) -> MotionResult {
        if !config.motion_enabled {
            return MotionResult {
                is_alert_triggered: false,
                motion_score: 0.0,
                consecutive_frames: 0,
                entity_type: "motion".to_string(),
                confidence: 0.0,
                bounding_box: None,
            };
        }

        let norm_rot = ((rotation_degrees % 360) + 360) % 360;
        if let Some(last_rot) = self.last_rotation {
            if last_rot != norm_rot {
                self.reset();
            }
        }
        self.last_rotation = Some(norm_rot);

        let min_interval = Duration::from_millis(1000 / config.analysis_fps.max(1) as u64);
        if let Some(last) = self.last_analysis_time {
            if now.duration_since(last) < min_interval {
                return MotionResult {
                    is_alert_triggered: false,
                    motion_score: 0.0,
                    consecutive_frames: self.consecutive_detections,
                    entity_type: "motion".to_string(),
                    confidence: 0.0,
                    bounding_box: None,
                };
            }
        }
        self.last_analysis_time = Some(now);

        Self::downsample_and_rotate_y(
            src_y,
            src_w,
            src_h,
            row_stride,
            rotation_degrees,
            &mut self.current_buffer,
        );

        let (oriented_w, oriented_h) = match norm_rot {
            90 | 270 => (src_h, src_w),
            _ => (src_w, src_h),
        };

        self.evaluate_motion_buffers(config, now, oriented_w, oriented_h)
    }

    /// Analyzes downscaled frame for motion differences.
    pub fn process_frame(
        &mut self,
        src_y: &[u8],
        src_w: usize,
        src_h: usize,
        row_stride: usize,
        config: &SurveillanceConfig,
        now: Instant,
    ) -> MotionResult {
        if !config.motion_enabled {
            return MotionResult {
                is_alert_triggered: false,
                motion_score: 0.0,
                consecutive_frames: 0,
                entity_type: "motion".to_string(),
                confidence: 0.0,
                bounding_box: None,
            };
        }

        // Frame rate limiter for motion analysis (e.g. 3-5 FPS)
        let min_interval = Duration::from_millis(1000 / config.analysis_fps.max(1) as u64);
        if let Some(last) = self.last_analysis_time {
            if now.duration_since(last) < min_interval {
                return MotionResult {
                    is_alert_triggered: false,
                    motion_score: 0.0,
                    consecutive_frames: self.consecutive_detections,
                    entity_type: "motion".to_string(),
                    confidence: 0.0,
                    bounding_box: None,
                };
            }
        }
        self.last_analysis_time = Some(now);

        // Downsample input into current_buffer
        Self::downsample_y_plane(
            src_y,
            src_w,
            src_h,
            row_stride,
            &mut self.current_buffer,
        );

        self.evaluate_motion_buffers(config, now, src_w, src_h)
    }

    fn evaluate_motion_buffers(
        &mut self,
        config: &SurveillanceConfig,
        now: Instant,
        oriented_w: usize,
        oriented_h: usize,
    ) -> MotionResult {
        if !self.has_reference_frame {
            self.previous_buffer.copy_from_slice(&self.current_buffer);
            self.has_reference_frame = true;
            return MotionResult {
                is_alert_triggered: false,
                motion_score: 0.0,
                consecutive_frames: 0,
                entity_type: "motion".to_string(),
                confidence: 0.0,
                bounding_box: None,
            };
        }

        // Bounding region check
        let (x_min, y_min, x_max, y_max) = if let Some([rx, ry, rw, rh]) = config.monitoring_region {
            (
                ((rx * ANALYSIS_WIDTH as f32).round() as usize).min(ANALYSIS_WIDTH),
                ((ry * ANALYSIS_HEIGHT as f32).round() as usize).min(ANALYSIS_HEIGHT),
                (((rx + rw) * ANALYSIS_WIDTH as f32).round() as usize).min(ANALYSIS_WIDTH),
                (((ry + rh) * ANALYSIS_HEIGHT as f32).round() as usize).min(ANALYSIS_HEIGHT),
            )
        } else {
            (0, 0, ANALYSIS_WIDTH, ANALYSIS_HEIGHT)
        };

        let mut changed_pixels: usize = 0;
        let mut total_pixels: usize = 0;
        let diff_threshold = config.pixel_diff_threshold;
        let mut block_counts = [0u16; 300];

        for y in y_min..y_max {
            let row_offset = y * ANALYSIS_WIDTH;
            let by = y / 8;
            for x in x_min..x_max {
                let idx = row_offset + x;
                let prev_p = self.previous_buffer[idx];
                let curr_p = self.current_buffer[idx];
                let diff = (prev_p as i16 - curr_p as i16).abs() as u8;

                if diff >= diff_threshold {
                    changed_pixels += 1;
                    let bx = x / 8;
                    block_counts[by * 20 + bx] += 1;
                }
                total_pixels += 1;
            }
        }

        // Update previous frame with current frame
        self.previous_buffer.copy_from_slice(&self.current_buffer);

        let motion_score = if total_pixels > 0 {
            changed_pixels as f32 / total_pixels as f32
        } else {
            0.0
        };

        // Active blocks analysis and spatial clustering
        let mut total_active_blocks = 0usize;
        for count in &block_counts {
            if *count >= 4 {
                total_active_blocks += 1;
            }
        }

        // 1. Global lighting / auto-exposure shift suppression:
        // When over 88% of blocks change simultaneously with a very high motion score,
        // it signifies a sudden global lighting change or auto-exposure adaptation rather
        // than a localized physical entity. Dynamically scales with monitoring region.
        let min_block_x = x_min / 8;
        let max_block_x = if x_max > x_min { ((x_max - 1) / 8) + 1 } else { 0 };
        let min_block_y = y_min / 8;
        let max_block_y = if y_max > y_min { ((y_max - 1) / 8) + 1 } else { 0 };
        let region_blocks = (max_block_x.saturating_sub(min_block_x)) * (max_block_y.saturating_sub(min_block_y));
        let required_active_blocks = if region_blocks >= 12 {
            ((region_blocks * 88) / 100).max(4)
        } else {
            265
        };

        let is_global_lighting_shift = total_active_blocks >= required_active_blocks && motion_score >= 0.85;

        #[derive(Debug, Clone)]
        struct MotionCluster {
            min_block_x: usize,
            max_block_x: usize,
            min_block_y: usize,
            max_block_y: usize,
            active_blocks: usize,
            pixel_diff_sum: u32,
        }

        let (raw_bbox, raw_entity, raw_confidence) = if is_global_lighting_shift {
            self.smoothed_bbox = None;
            self.tracking_miss_count = 0;
            self.person_hold_frames = 0;
            (None, "motion".to_string(), 0.0)
        } else if total_active_blocks > 0 {
            // 2. Spatial Clustering using 8-connected component flood-fill:
            // Prevents isolated corner noise dots from expanding bounding boxes across the full frame.
            let mut visited = [false; 300];
            let mut clusters: Vec<MotionCluster> = Vec::with_capacity(8);

            for start_by in 0..15 {
                for start_bx in 0..20 {
                    let start_idx = start_by * 20 + start_bx;
                    if visited[start_idx] || block_counts[start_idx] < 4 {
                        continue;
                    }

                    let mut queue = [0u16; 300];
                    let mut head = 0usize;
                    let mut tail = 0usize;

                    queue[tail] = start_idx as u16;
                    tail += 1;
                    visited[start_idx] = true;

                    let mut min_bx = start_bx;
                    let mut max_bx = start_bx;
                    let mut min_by = start_by;
                    let mut max_by = start_by;
                    let mut cluster_blocks = 0usize;
                    let mut cluster_diff_sum = 0u32;

                    while head < tail {
                        let curr_idx = queue[head] as usize;
                        head += 1;

                        let cbx = curr_idx % 20;
                        let cby = curr_idx / 20;

                        cluster_blocks += 1;
                        cluster_diff_sum += block_counts[curr_idx] as u32;

                        if cbx < min_bx { min_bx = cbx; }
                        if cbx > max_bx { max_bx = cbx; }
                        if cby < min_by { min_by = cby; }
                        if cby > max_by { max_by = cby; }

                        let min_ny = cby.saturating_sub(1);
                        let max_ny = (cby + 1).min(14);
                        let min_nx = cbx.saturating_sub(1);
                        let max_nx = (cbx + 1).min(19);

                        for ny in min_ny..=max_ny {
                            for nx in min_nx..=max_nx {
                                let n_idx = ny * 20 + nx;
                                if !visited[n_idx] && block_counts[n_idx] >= 4 {
                                    visited[n_idx] = true;
                                    queue[tail] = n_idx as u16;
                                    tail += 1;
                                }
                            }
                        }
                    }

                    clusters.push(MotionCluster {
                        min_block_x: min_bx,
                        max_block_x: max_bx,
                        min_block_y: min_by,
                        max_block_y: max_by,
                        active_blocks: cluster_blocks,
                        pixel_diff_sum: cluster_diff_sum,
                    });
                }
            }

            // Bridge nearby clusters (distance <= 1 block) to combine contiguous entity parts
            let mut merged = true;
            while merged && clusters.len() > 1 {
                merged = false;
                'outer: for i in 0..clusters.len() {
                    for j in (i + 1)..clusters.len() {
                        let c1 = &clusters[i];
                        let c2 = &clusters[j];

                        let gap_x = if c1.max_block_x < c2.min_block_x {
                            c2.min_block_x - c1.max_block_x - 1
                        } else if c2.max_block_x < c1.min_block_x {
                            c1.min_block_x - c2.max_block_x - 1
                        } else {
                            0
                        };

                        let gap_y = if c1.max_block_y < c2.min_block_y {
                            c2.min_block_y - c1.max_block_y - 1
                        } else if c2.max_block_y < c1.min_block_y {
                            c1.min_block_y - c2.max_block_y - 1
                        } else {
                            0
                        };

                        if gap_x <= 1 && gap_y <= 1 {
                            let combined = MotionCluster {
                                min_block_x: c1.min_block_x.min(c2.min_block_x),
                                max_block_x: c1.max_block_x.max(c2.max_block_x),
                                min_block_y: c1.min_block_y.min(c2.min_block_y),
                                max_block_y: c1.max_block_y.max(c2.max_block_y),
                                active_blocks: c1.active_blocks + c2.active_blocks,
                                pixel_diff_sum: c1.pixel_diff_sum + c2.pixel_diff_sum,
                            };
                            clusters.remove(j);
                            clusters[i] = combined;
                            merged = true;
                            break 'outer;
                        }
                    }
                }
            }

            // Select the dominant primary motion cluster
            let dominant_cluster = clusters.into_iter().max_by_key(|c| (c.active_blocks, c.pixel_diff_sum));

            if let Some(dominant) = dominant_cluster {
                let min_bx = dominant.min_block_x;
                let max_bx = dominant.max_block_x;
                let min_by = dominant.min_block_y;
                let max_by = dominant.max_block_y;
                let cluster_blocks = dominant.active_blocks;

                let bbox_x = (min_bx * 8) as f32 / ANALYSIS_WIDTH as f32;
                let bbox_y = (min_by * 8) as f32 / ANALYSIS_HEIGHT as f32;
                let bbox_w = (((max_bx - min_bx + 1) * 8) as f32 / ANALYSIS_WIDTH as f32).min(1.0 - bbox_x);
                let bbox_h = (((max_by - min_by + 1) * 8) as f32 / ANALYSIS_HEIGHT as f32).min(1.0 - bbox_y);
                let bbox = Some([bbox_x, bbox_y, bbox_w, bbox_h]);

                let pixel_w = (max_bx - min_bx + 1) * 8;
                let pixel_h = (max_by - min_by + 1) * 8;
                let aspect_ratio = (bbox_h * oriented_h as f32) / (bbox_w * oriented_w as f32).max(1e-5);
                let box_blocks = (max_bx - min_bx + 1) * (max_by - min_by + 1);
                let block_density = cluster_blocks as f32 / box_blocks.max(1) as f32;
                let frame_area_ratio = (pixel_w * pixel_h) as f32 / (ANALYSIS_WIDTH * ANALYSIS_HEIGHT) as f32;

                let span_y = max_by - min_by + 1;
                let mut top_blocks = 0usize;
                let mut mid_blocks = 0usize;
                let mut bot_blocks = 0usize;

                for by in min_by..=max_by {
                    let rel_y = by - min_by;
                    let zone = if span_y >= 3 {
                        ((rel_y as f32 * 3.0) / span_y as f32).floor() as usize
                    } else if rel_y == 0 {
                        0
                    } else {
                        1
                    };

                    for bx in min_bx..=max_bx {
                        if block_counts[by * 20 + bx] >= 4 {
                            match zone {
                                0 => top_blocks += 1,
                                1 => mid_blocks += 1,
                                _ => bot_blocks += 1,
                            }
                        }
                    }
                }

                let vertical_zones_active = (top_blocks > 0) as u8
                    + (mid_blocks > 0) as u8
                    + (bot_blocks > 0) as u8;

                let is_person = aspect_ratio >= 1.2
                    && aspect_ratio <= 3.8
                    && bbox_h >= 0.15
                    && bbox_w <= 0.85
                    && vertical_zones_active >= 2
                    && block_density >= 0.08
                    && block_density <= 1.0
                    && frame_area_ratio >= 0.015
                    && frame_area_ratio <= 0.85;

                let (ent, conf) = if is_person {
                    let mut c = 0.70f32;
                    if aspect_ratio >= 1.4 && aspect_ratio <= 2.8 {
                        c += 0.12;
                    }
                    if vertical_zones_active == 3 {
                        c += 0.10;
                    }
                    if bbox_h >= 0.25 {
                        c += 0.05;
                    }
                    ("person".to_string(), c.clamp(0.65, 0.96))
                } else if cluster_blocks >= 2 && frame_area_ratio >= 0.01 && frame_area_ratio <= 0.75 && block_density >= 0.10 {
                    let mut c = 0.62f32;
                    if block_density >= 0.20 {
                        c += 0.12;
                    }
                    if frame_area_ratio >= 0.025 {
                        c += 0.10;
                    }
                    ("object".to_string(), c.clamp(0.60, 0.90))
                } else {
                    ("motion".to_string(), (motion_score * 2.5).clamp(0.40, 0.85))
                };

                (bbox, ent, conf)
            } else {
                (None, "motion".to_string(), (motion_score * 2.5).clamp(0.0, 0.50))
            }
        } else {
            (None, "motion".to_string(), (motion_score * 2.5).clamp(0.0, 0.50))
        };

        // 3. Temporal Smoothing via Exponential Moving Average (EMA) and Hysteresis:
        // Smooths bounding box coordinates between frames to eliminate jitter.
        let bounding_box = if let Some(raw_box) = raw_bbox {
            if let Some(prev_box) = self.smoothed_bbox {
                let raw_center_x = raw_box[0] + raw_box[2] * 0.5;
                let raw_center_y = raw_box[1] + raw_box[3] * 0.5;
                let prev_center_x = prev_box[0] + prev_box[2] * 0.5;
                let prev_center_y = prev_box[1] + prev_box[3] * 0.5;
                let distance = (((raw_center_x - prev_center_x).powi(2)) + ((raw_center_y - prev_center_y).powi(2))).sqrt();

                if distance < 0.35 {
                    let alpha = 0.45f32;
                    let smoothed = [
                        alpha * raw_box[0] + (1.0 - alpha) * prev_box[0],
                        alpha * raw_box[1] + (1.0 - alpha) * prev_box[1],
                        alpha * raw_box[2] + (1.0 - alpha) * prev_box[2],
                        alpha * raw_box[3] + (1.0 - alpha) * prev_box[3],
                    ];
                    self.smoothed_bbox = Some(smoothed);
                    self.tracking_miss_count = 0;
                    Some(smoothed)
                } else {
                    self.smoothed_bbox = Some(raw_box);
                    self.tracking_miss_count = 0;
                    Some(raw_box)
                }
            } else {
                self.smoothed_bbox = Some(raw_box);
                self.tracking_miss_count = 0;
                Some(raw_box)
            }
        } else if let Some(prev_box) = self.smoothed_bbox {
            self.tracking_miss_count += 1;
            if self.tracking_miss_count <= 2 {
                Some(prev_box)
            } else {
                self.smoothed_bbox = None;
                None
            }
        } else {
            None
        };

        // 4. Entity Label Stability (Hysteresis):
        // Hold the "person" label steady for several frames if a person pauses or turns,
        // avoiding rapid flickering between "person" and "object" / "motion".
        let (entity_type, confidence_val) = if raw_entity == "person" {
            self.stable_entity = "person".to_string();
            self.person_hold_frames = 4;
            ("person".to_string(), raw_confidence)
        } else if self.person_hold_frames > 0 && bounding_box.is_some() {
            self.person_hold_frames -= 1;
            let held_conf = (raw_confidence.max(0.68) * 0.96).min(0.92);
            ("person".to_string(), held_conf)
        } else {
            self.stable_entity = raw_entity.clone();
            self.person_hold_frames = 0;
            (raw_entity, raw_confidence)
        };

        // Smooth confidence score
        let confidence = if bounding_box.is_some() && self.smoothed_confidence > 0.0 {
            let alpha_conf = 0.40f32;
            let smoothed_conf = alpha_conf * confidence_val + (1.0 - alpha_conf) * self.smoothed_confidence;
            self.smoothed_confidence = smoothed_conf;
            smoothed_conf
        } else {
            self.smoothed_confidence = confidence_val;
            confidence_val
        };

        let is_motion_detected = !is_global_lighting_shift && (motion_score >= config.min_area_ratio);

        let mut alert_triggered = false;
        if is_motion_detected {
            self.consecutive_detections += 1;

            if self.consecutive_detections >= config.min_consecutive_frames {
                // Check cooldown
                let in_cooldown = if let Some(last_alert) = self.last_alert_time {
                    now.duration_since(last_alert) < Duration::from_secs(config.cooldown_seconds)
                } else {
                    false
                };

                if !in_cooldown {
                    alert_triggered = true;
                    self.last_alert_time = Some(now);
                }
            }
        } else {
            self.consecutive_detections = self.consecutive_detections.saturating_sub(1);
        }

        MotionResult {
            is_alert_triggered: alert_triggered,
            motion_score,
            consecutive_frames: self.consecutive_detections,
            entity_type,
            confidence,
            bounding_box,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_frame(w: usize, h: usize, fill_val: u8) -> Vec<u8> {
        vec![fill_val; w * h]
    }

    #[test]
    fn test_downsample_and_rotate_y() {
        let w = 320;
        let h = 240;
        let mut src = vec![0u8; w * h];
        // Populate 4 quadrants with distinct values:
        // Top-Left: 10, Top-Right: 20, Bottom-Left: 30, Bottom-Right: 40
        for y in 0..h {
            for x in 0..w {
                let val = match (x < w / 2, y < h / 2) {
                    (true, true) => 10,
                    (false, true) => 20,
                    (true, false) => 30,
                    (false, false) => 40,
                };
                src[y * w + x] = val;
            }
        }

        let mut dst = vec![0u8; ANALYSIS_WIDTH * ANALYSIS_HEIGHT];

        // Sample points inside the 4 quadrants of dst (160x120):
        let p_tl = 30 * ANALYSIS_WIDTH + 40;
        let p_tr = 30 * ANALYSIS_WIDTH + 120;
        let p_bl = 90 * ANALYSIS_WIDTH + 40;
        let p_br = 90 * ANALYSIS_WIDTH + 120;

        // 0 degrees: Identity
        MotionDetector::downsample_and_rotate_y(&src, w, h, w, 0, &mut dst);
        assert_eq!(dst[p_tl], 10, "0 deg top-left");
        assert_eq!(dst[p_tr], 20, "0 deg top-right");
        assert_eq!(dst[p_bl], 30, "0 deg bottom-left");
        assert_eq!(dst[p_br], 40, "0 deg bottom-right");

        // 90 degrees clockwise:
        // Original bottom-left (30) becomes rotated top-left
        // Original top-left (10) becomes rotated top-right
        // Original bottom-right (40) becomes rotated bottom-left
        // Original top-right (20) becomes rotated bottom-right
        MotionDetector::downsample_and_rotate_y(&src, w, h, w, 90, &mut dst);
        assert_eq!(dst[p_tl], 30, "90 deg top-left");
        assert_eq!(dst[p_tr], 10, "90 deg top-right");
        assert_eq!(dst[p_bl], 40, "90 deg bottom-left");
        assert_eq!(dst[p_br], 20, "90 deg bottom-right");

        // 180 degrees:
        // Original bottom-right (40) becomes rotated top-left
        // Original bottom-left (30) becomes rotated top-right
        // Original top-right (20) becomes rotated bottom-left
        // Original top-left (10) becomes rotated bottom-right
        MotionDetector::downsample_and_rotate_y(&src, w, h, w, 180, &mut dst);
        assert_eq!(dst[p_tl], 40, "180 deg top-left");
        assert_eq!(dst[p_tr], 30, "180 deg top-right");
        assert_eq!(dst[p_bl], 20, "180 deg bottom-left");
        assert_eq!(dst[p_br], 10, "180 deg bottom-right");

        // 270 degrees clockwise:
        // Original top-right (20) becomes rotated top-left
        // Original bottom-right (40) becomes rotated top-right
        // Original top-left (10) becomes rotated bottom-left
        // Original bottom-left (30) becomes rotated bottom-right
        MotionDetector::downsample_and_rotate_y(&src, w, h, w, 270, &mut dst);
        assert_eq!(dst[p_tl], 20, "270 deg top-left");
        assert_eq!(dst[p_tr], 40, "270 deg top-right");
        assert_eq!(dst[p_bl], 10, "270 deg bottom-left");
        assert_eq!(dst[p_br], 30, "270 deg bottom-right");

        // Padded row stride check
        let stride = w + 32;
        let mut padded_src = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                padded_src[y * stride + x] = src[y * w + x];
            }
        }
        MotionDetector::downsample_and_rotate_y(&padded_src, w, h, stride, 180, &mut dst);
        assert_eq!(dst[p_tl], 40, "180 deg padded stride top-left");
        assert_eq!(dst[p_tr], 30, "180 deg padded stride top-right");
        assert_eq!(dst[p_bl], 20, "180 deg padded stride bottom-left");
        assert_eq!(dst[p_br], 10, "180 deg padded stride bottom-right");
    }

    #[test]
    fn test_motion_detector_rotation_change_resets_reference_frame() {
        let mut detector = MotionDetector::new();
        let config = SurveillanceConfig::default();
        let mut now = Instant::now();

        let w = 640;
        let h = 480;
        // Asymmetric frame: top half 50, bottom half 200
        let mut frame = vec![50u8; w * h];
        for y in (h / 2)..h {
            for x in 0..w {
                frame[y * w + x] = 200;
            }
        }

        // Frame 1 at 0 degrees sets reference frame
        let res1 = detector.process_raw_frame(&frame, w, h, w, 0, &config, now);
        assert!(!res1.is_alert_triggered);
        assert_eq!(res1.motion_score, 0.0);

        // Frame 2 at 180 degrees:
        // Because rotation changed from 0 to 180, detector resets reference frame
        // instead of false-alarming on 180-degree inverted pixels
        now += Duration::from_millis(300);
        let res2 = detector.process_raw_frame(&frame, w, h, w, 180, &config, now);
        assert!(!res2.is_alert_triggered);
        assert_eq!(res2.motion_score, 0.0, "Rotation change must reset reference frame without false alert");
    }

    #[test]
    fn test_portrait_mode_aspect_ratio_person_detection() {
        let mut detector = MotionDetector::new();
        let config = SurveillanceConfig::default();
        let mut now = Instant::now();

        let w = 640;
        let h = 480;
        let frame_empty = vec![100u8; w * h];

        // Frame 1: empty reference frame at 90 degrees
        detector.process_raw_frame(&frame_empty, w, h, w, 90, &config, now);

        // Frame 2: simulate a person standing in the oriented 480x640 frame
        // In 90 degrees orientation, oriented_w = 480, oriented_h = 640.
        // A vertical person silhouette: width 64px, height 144px (aspect ratio 2.25)
        let mut frame_person = frame_empty.clone();
        // In 90 deg: orig_x = out_y, orig_y = h - 1 - out_x
        for out_y in 200..344 {
            for out_x in 200..264 {
                let orig_x = out_y;
                let orig_y = h - 1 - out_x;
                frame_person[orig_y * w + orig_x] = 220;
            }
        }

        now += Duration::from_millis(300);
        let res = detector.process_raw_frame(&frame_person, w, h, w, 90, &config, now);
        assert_eq!(res.entity_type, "person", "Person standing in portrait orientation must be recognized as person");
        assert!(res.confidence >= 0.70, "Confidence should exceed 70% for upright person");
    }

    #[test]
    fn test_motion_detector_no_motion_static_frame() {
        let mut detector = MotionDetector::new();
        let config = SurveillanceConfig::default();
        let mut now = Instant::now();

        let frame = create_test_frame(640, 480, 128);

        // Frame 1: Reference setup
        let res1 = detector.process_frame(&frame, 640, 480, 640, &config, now);
        assert!(!res1.is_alert_triggered);
        assert_eq!(res1.motion_score, 0.0);

        // Frame 2: Identical frame -> score 0.0
        now += Duration::from_millis(300);
        let res2 = detector.process_frame(&frame, 640, 480, 640, &config, now);
        assert!(!res2.is_alert_triggered);
        assert_eq!(res2.motion_score, 0.0);
        assert_eq!(res2.consecutive_frames, 0);
    }

    #[test]
    fn test_motion_detector_consecutive_frame_confirmation() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 2;
        config.cooldown_seconds = 5;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        let mut frame_moving_a = frame_empty.clone();
        for y in 100..300 {
            for x in 100..300 {
                frame_moving_a[y * 640 + x] = 200;
            }
        }
        let mut frame_moving_b = frame_empty.clone();
        for y in 100..300 {
            for x in 250..450 {
                frame_moving_b[y * 640 + x] = 200;
            }
        }

        // Frame 1: init reference with empty frame
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: moving entity appears (First detection frame)
        now += Duration::from_millis(300);
        let res2 = detector.process_frame(&frame_moving_a, 640, 480, 640, &config, now);
        assert!(res2.motion_score > 0.05);
        assert_eq!(res2.consecutive_frames, 1);
        // Alert should NOT trigger yet because min_consecutive_frames = 2
        assert!(!res2.is_alert_triggered);

        // Frame 3: moving entity moves further (Second consecutive motion frame)
        now += Duration::from_millis(300);
        let res3 = detector.process_frame(&frame_moving_b, 640, 480, 640, &config, now);
        assert!(res3.motion_score > 0.05);
        assert_eq!(res3.consecutive_frames, 2);
        // Now alert MUST trigger!
        assert!(res3.is_alert_triggered);
    }

    #[test]
    fn test_motion_detector_alert_cooldown() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        config.cooldown_seconds = 5;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        let mut frame_motion = frame_empty.clone();
        for y in 100..300 {
            for x in 150..350 {
                frame_motion[y * 640 + x] = 220;
            }
        }

        // Frame 1: reference
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: Motion -> Triggers alert
        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_motion, 640, 480, 640, &config, now);
        assert!(res.is_alert_triggered);

        // Frame 3 (2 seconds later, within 5s cooldown): Motion -> Cooldown suppresses alert
        now += Duration::from_secs(2);
        let res = detector.process_frame(&frame_empty, 640, 480, 640, &config, now);
        assert!(res.motion_score > 0.05);
        assert!(!res.is_alert_triggered, "Alert should be suppressed during cooldown");

        // Frame 4 (6 seconds later, past 5s cooldown): Motion -> Alert triggers again
        now += Duration::from_secs(6);
        let res = detector.process_frame(&frame_motion, 640, 480, 640, &config, now);
        assert!(res.is_alert_triggered, "Alert should trigger after cooldown expired");
    }

    #[test]
    fn test_noise_below_threshold_ignored() {
        let mut detector = MotionDetector::new();
        let config = SurveillanceConfig::default(); // default pixel_diff_threshold ~ 25
        let mut now = Instant::now();

        let frame1 = create_test_frame(640, 480, 100);
        let frame2 = create_test_frame(640, 480, 105); // Only 5 diff (camera noise)

        detector.process_frame(&frame1, 640, 480, 640, &config, now);
        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame2, 640, 480, 640, &config, now);

        assert_eq!(res.motion_score, 0.0);
        assert!(!res.is_alert_triggered);
    }

    #[test]
    fn test_motion_detection_disabled_produces_no_alerts_and_zero_score() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.motion_enabled = false;
        let mut now = Instant::now();

        let frame_white = create_test_frame(640, 480, 255);
        let frame_black = create_test_frame(640, 480, 0);

        // Frame 1
        let res1 = detector.process_raw_frame(&frame_white, 640, 480, 640, 0, &config, now);
        assert!(!res1.is_alert_triggered);
        assert_eq!(res1.motion_score, 0.0);
        assert_eq!(res1.consecutive_frames, 0);

        // Frame 2 (huge change white to black, but motion_enabled = false)
        now += Duration::from_millis(300);
        let res2 = detector.process_raw_frame(&frame_black, 640, 480, 640, 0, &config, now);
        assert!(!res2.is_alert_triggered);
        assert_eq!(res2.motion_score, 0.0);
        assert_eq!(res2.consecutive_frames, 0);

        // Frame 3 (another huge change black to white, still motion_enabled = false)
        now += Duration::from_millis(300);
        let res3 = detector.process_raw_frame(&frame_white, 640, 480, 640, 0, &config, now);
        assert!(!res3.is_alert_triggered);
        assert_eq!(res3.motion_score, 0.0);
        assert_eq!(res3.consecutive_frames, 0);
    }

    #[test]
    fn test_classify_person_silhouette() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        // 640x480 frame
        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: Introduce an upright person-like silhouette (width ~ 160, height ~ 360, aspect_ratio ~ 2.25)
        // in downsampled 160x120 coordinates: x ~ 60..100 (width 40), y ~ 20..100 (height 80)
        let mut frame_person = frame_empty.clone();
        for y in 80..400 {
            for x in 240..400 {
                frame_person[y * 640 + x] = 220;
            }
        }

        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_person, 640, 480, 640, &config, now);
        assert!(res.is_alert_triggered);
        assert_eq!(res.entity_type, "person");
        assert!(res.confidence >= 0.70);
        assert!(res.bounding_box.is_some());
        let [_bx, _by, bw, bh] = res.bounding_box.unwrap();
        assert!(bw > 0.1 && bw < 0.5);
        assert!(bh > 0.4 && bh < 0.9);
        assert!(bh > bw);
    }

    #[test]
    fn test_classify_object_silhouette() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: Introduce a horizontal object (width ~ 320, height ~ 100, aspect_ratio ~ 0.31, e.g. vehicle/pet)
        let mut frame_object = frame_empty.clone();
        for y in 260..360 {
            for x in 160..480 {
                frame_object[y * 640 + x] = 220;
            }
        }

        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_object, 640, 480, 640, &config, now);
        assert!(res.is_alert_triggered);
        assert_eq!(res.entity_type, "object");
        assert!(res.confidence >= 0.60);
        assert!(res.bounding_box.is_some());
        let [_bx, _by, bw, bh] = res.bounding_box.unwrap();
        assert!(bw > bh);
    }

    #[test]
    fn test_classify_person_compact_silhouette() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Standing person occupying 4 block rows (span_y = 4, 32 pixels in 120-height frame => y 160..300 in 480 frame)
        // width ~ 70 pixels (aspect ratio ~ 2.0)
        let mut frame_person = frame_empty.clone();
        for y in 160..300 {
            for x in 280..350 {
                frame_person[y * 640 + x] = 220;
            }
        }

        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_person, 640, 480, 640, &config, now);
        assert!(res.is_alert_triggered);
        assert_eq!(res.entity_type, "person");
        assert!(res.confidence >= 0.70);
        assert!(res.bounding_box.is_some());
    }

    #[test]
    fn test_global_lighting_shift_suppresses_full_screen_box() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_dark = create_test_frame(640, 480, 40);
        let frame_bright = create_test_frame(640, 480, 220);

        // Frame 1: baseline reference
        detector.process_frame(&frame_dark, 640, 480, 640, &config, now);

        // Frame 2: sudden whole-screen exposure / lighting switch
        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_bright, 640, 480, 640, &config, now);

        assert!(res.motion_score > 0.85);
        // Suppress full-screen bounding box on global lighting shifts
        assert!(res.bounding_box.is_none(), "Global illumination shifts should not produce a full-screen bounding box");
        assert_eq!(res.entity_type, "motion", "Global lighting shift must not be classified as a person or object");
        assert!(!res.is_alert_triggered, "Global illumination shifts must not trigger false alarms");
        assert_eq!(res.consecutive_frames, 0, "Global illumination shifts must not increment consecutive motion frames");
    }

    #[test]
    fn test_global_lighting_shift_with_monitoring_region() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.monitoring_region = Some([0.1, 0.1, 0.5, 0.5]);
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_dark = create_test_frame(640, 480, 40);
        let frame_bright = create_test_frame(640, 480, 220);

        detector.process_frame(&frame_dark, 640, 480, 640, &config, now);

        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_bright, 640, 480, 640, &config, now);

        assert!(res.motion_score > 0.85);
        assert!(res.bounding_box.is_none(), "Global illumination shift in monitoring region must suppress bounding box");
        assert_eq!(res.entity_type, "motion");
        assert!(!res.is_alert_triggered, "Global illumination shift in monitoring region must not trigger false alarm");
        assert_eq!(res.consecutive_frames, 0);
    }

    #[test]
    fn test_spatial_clustering_separates_distant_noise_corners() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: Introduce two isolated noise patches at opposite corners
        // Top-left corner (0..32, 0..32) and bottom-right corner (600..640, 440..480)
        let mut frame_noise = frame_empty.clone();
        for y in 0..32 {
            for x in 0..32 {
                frame_noise[y * 640 + x] = 220;
            }
        }
        for y in 448..480 {
            for x in 608..640 {
                frame_noise[y * 640 + x] = 220;
            }
        }

        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_noise, 640, 480, 640, &config, now);

        assert!(res.bounding_box.is_some());
        let [_bx, _by, bw, bh] = res.bounding_box.unwrap();
        // Bounding box must NOT span the entire screen (should isolate the dominant noise cluster)
        assert!(bw <= 0.25, "Bounding box width {} should isolate local cluster and not span entire screen", bw);
        assert!(bh <= 0.25, "Bounding box height {} should isolate local cluster and not span entire screen", bh);
    }

    #[test]
    fn test_spatial_clustering_isolates_person_from_distant_noise() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: Upright person in center PLUS isolated noise in far corner
        let mut frame_person_and_noise = frame_empty.clone();
        // Person in center
        for y in 80..400 {
            for x in 240..400 {
                frame_person_and_noise[y * 640 + x] = 220;
            }
        }
        // Distant noise in bottom-right corner
        for y in 448..480 {
            for x in 608..640 {
                frame_person_and_noise[y * 640 + x] = 220;
            }
        }

        now += Duration::from_millis(300);
        let res = detector.process_frame(&frame_person_and_noise, 640, 480, 640, &config, now);

        assert_eq!(res.entity_type, "person");
        assert!(res.bounding_box.is_some());
        let [bx, _by, bw, bh] = res.bounding_box.unwrap();
        // Must bound the person cluster and not stretch to the bottom-right corner
        assert!(bw < 0.50, "Bounding box width {} must not stretch to corner noise", bw);
        assert!(bh < 0.90, "Bounding box height {} must not stretch to corner noise", bh);
        assert!(bx >= 0.30 && bx <= 0.45, "Bounding box x {} must center on the person", bx);
    }

    #[test]
    fn test_temporal_ema_smoothing_trajectory() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 1: Person at position X = 160..320
        let mut frame_pos1 = frame_empty.clone();
        for y in 80..400 {
            for x in 160..320 {
                frame_pos1[y * 640 + x] = 220;
            }
        }
        now += Duration::from_millis(250);
        let res1 = detector.process_frame(&frame_pos1, 640, 480, 640, &config, now);
        let box1 = res1.bounding_box.expect("First detection must produce box");

        // Frame 2: Person steps to the right at X = 240..400
        let mut frame_pos2 = frame_empty.clone();
        for y in 80..400 {
            for x in 240..400 {
                frame_pos2[y * 640 + x] = 220;
            }
        }
        now += Duration::from_millis(250);
        let res2 = detector.process_frame(&frame_pos2, 640, 480, 640, &config, now);
        let box2 = res2.bounding_box.expect("Second detection must produce box");

        // The smoothed X coordinate must be between box1 X and the raw new X
        let raw_target_x = (240.0 * (ANALYSIS_WIDTH as f32 / 640.0)) / ANALYSIS_WIDTH as f32;
        assert!(box2[0] > box1[0], "Smoothed box must move toward the right");
        assert!(box2[0] < raw_target_x + 0.05, "Smoothed box must interpolate smoothly without overshooting");
    }

    #[test]
    fn test_entity_label_stability_hysteresis() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: Clear upright person silhouette
        let mut frame_person = frame_empty.clone();
        for y in 80..400 {
            for x in 240..400 {
                frame_person[y * 640 + x] = 220;
            }
        }
        now += Duration::from_millis(250);
        let res1 = detector.process_frame(&frame_person, 640, 480, 640, &config, now);
        assert_eq!(res1.entity_type, "person");

        // Frame 3: Person momentarily bends or limbs spread, changing aspect ratio to horizontal
        let mut frame_transient = frame_empty.clone();
        for y in 260..360 {
            for x in 160..480 {
                frame_transient[y * 640 + x] = 220;
            }
        }
        now += Duration::from_millis(250);
        let res2 = detector.process_frame(&frame_transient, 640, 480, 640, &config, now);

        // Due to entity label stability hysteresis, label remains "person" instead of flickering!
        assert_eq!(res2.entity_type, "person", "Hysteresis must stabilize person label across transient pose shifts");
    }

    #[test]
    fn test_entity_label_stability_preserved_during_momentary_pause() {
        let mut detector = MotionDetector::new();
        let mut config = SurveillanceConfig::default();
        config.min_consecutive_frames = 1;
        let mut now = Instant::now();

        let frame_empty = create_test_frame(640, 480, 50);
        detector.process_frame(&frame_empty, 640, 480, 640, &config, now);

        // Frame 2: Person walking (height ~ 320, width ~ 160)
        let mut frame_person = frame_empty.clone();
        for y in 80..400 {
            for x in 240..400 {
                frame_person[y * 640 + x] = 220;
            }
        }
        now += Duration::from_millis(250);
        let res1 = detector.process_frame(&frame_person, 640, 480, 640, &config, now);
        assert_eq!(res1.entity_type, "person");
        assert!(res1.bounding_box.is_some());

        // Frame 3: Person pauses momentarily (identical to frame 2 => zero frame difference)
        now += Duration::from_millis(250);
        let res2 = detector.process_frame(&frame_person, 640, 480, 640, &config, now);
        // Bounding box must be held via temporal hysteresis
        assert!(res2.bounding_box.is_some(), "Bounding box should be held during momentary pause");
        // Entity label MUST stay 'person' instead of flickering to 'motion'!
        assert_eq!(res2.entity_type, "person", "Person label must not flicker to motion when person pauses");

        // Frame 4: Person remains paused (second miss frame)
        now += Duration::from_millis(250);
        let res3 = detector.process_frame(&frame_person, 640, 480, 640, &config, now);
        assert!(res3.bounding_box.is_some(), "Bounding box should continue to be held on second miss frame");
        assert_eq!(res3.entity_type, "person", "Person label must be retained across pause hysteresis");

        // Frame 5: Person paused for 3rd miss frame => tracking expired
        now += Duration::from_millis(250);
        let res4 = detector.process_frame(&frame_person, 640, 480, 640, &config, now);
        assert!(res4.bounding_box.is_none(), "Bounding box should expire after exceeding miss threshold");
    }

    #[test]
    fn test_update_active_detection_state_transitions() {
        let mut detector = MotionDetector::new();
        // Initially no active detection
        assert!(!detector.update_active_detection_state(false));
        // Becomes active -> returns false (no clear signal)
        assert!(!detector.update_active_detection_state(true));
        // Stays active -> returns false
        assert!(!detector.update_active_detection_state(true));
        // Transitions to inactive -> returns true (clear signal!)
        assert!(detector.update_active_detection_state(false));
        // Stays inactive -> returns false
        assert!(!detector.update_active_detection_state(false));
    }
}
