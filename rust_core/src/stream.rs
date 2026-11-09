use parking_lot::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch};

#[derive(Clone)]
pub struct FramePayload {
    pub jpeg_data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub timestamp_millis: u64,
    pub frame_seq: u64,
}

pub struct StreamStats {
    pub viewer_count: Arc<AtomicUsize>,
    pub actual_fps_x10: Arc<AtomicU32>,
    pub dropped_frames: Arc<AtomicU64>,
    pub total_encoded_frames: Arc<AtomicU64>,
}

#[derive(Clone)]
pub struct FrameBroadcaster {
    sender: Arc<watch::Sender<Option<FramePayload>>>,
    receiver: watch::Receiver<Option<FramePayload>>,
    last_frame: Arc<RwLock<Option<FramePayload>>>,
    stats: Arc<StreamStats>,
    last_encode_time: Arc<RwLock<Option<Instant>>>,
    fps_counter: Arc<RwLock<(Instant, u32)>>,
}

impl Default for FrameBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameBroadcaster {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(None);
        Self {
            sender: Arc::new(sender),
            receiver,
            last_frame: Arc::new(RwLock::new(None)),
            stats: Arc::new(StreamStats {
                viewer_count: Arc::new(AtomicUsize::new(0)),
                actual_fps_x10: Arc::new(AtomicU32::new(0)),
                dropped_frames: Arc::new(AtomicU64::new(0)),
                total_encoded_frames: Arc::new(AtomicU64::new(0)),
            }),
            last_encode_time: Arc::new(RwLock::new(None)),
            fps_counter: Arc::new(RwLock::new((Instant::now(), 0))),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<FramePayload>> {
        self.receiver.clone()
    }

    pub fn get_latest_frame(&self) -> Option<FramePayload> {
        self.last_frame.read().clone()
    }

    pub fn increment_viewer(&self) {
        self.stats.viewer_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_viewer(&self) {
        self.stats.viewer_count.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn get_viewer_count(&self) -> usize {
        self.stats.viewer_count.load(Ordering::Relaxed)
    }

    pub fn get_actual_fps(&self) -> f32 {
        self.stats.actual_fps_x10.load(Ordering::Relaxed) as f32 / 10.0
    }

    pub fn get_dropped_frames(&self) -> u64 {
        self.stats.dropped_frames.load(Ordering::Relaxed)
    }

    pub fn get_total_encoded_frames(&self) -> u64 {
        self.stats.total_encoded_frames.load(Ordering::Relaxed)
    }

    /// Checks if a frame should be encoded based on target FPS with adaptive jitter tolerance.
    pub fn should_encode_frame(&self, target_fps: u32, now: Instant) -> bool {
        if target_fps <= 1 {
            let mut last_time = self.last_encode_time.write();
            if let Some(prev) = *last_time {
                if now.duration_since(prev) < Duration::from_millis(900) {
                    return false;
                }
            }
            *last_time = Some(now);
            return true;
        }

        let interval_ms = 1000 / target_fps as u64;
        // Adaptive jitter tolerance: allows frames within 30% timing variance to prevent sampling aliasing/halving
        let min_interval = Duration::from_millis(interval_ms.saturating_sub(15).max(20));
        let mut last_time = self.last_encode_time.write();
        if let Some(prev) = *last_time {
            if now.duration_since(prev) < min_interval {
                self.stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        }
        *last_time = Some(now);
        true
    }

    /// Broadcasts a newly encoded JPEG frame to all connected viewers.
    pub fn broadcast_frame(
        &self,
        jpeg_bytes: Vec<u8>,
        width: u32,
        height: u32,
        timestamp_millis: u64,
    ) {
        let seq = self.stats.total_encoded_frames.fetch_add(1, Ordering::Relaxed) + 1;
        let payload = FramePayload {
            jpeg_data: Arc::new(jpeg_bytes),
            width,
            height,
            timestamp_millis,
            frame_seq: seq,
        };

        // Update latest frame cache
        *self.last_frame.write() = Some(payload.clone());

        // Notify all watch subscribers (discarding outdated ones automatically)
        let _ = self.sender.send(Some(payload));

        // Update FPS calculation
        let mut fps_lock = self.fps_counter.write();
        fps_lock.1 += 1;
        let elapsed = fps_lock.0.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let fps = (fps_lock.1 as f32 / elapsed.as_secs_f32()) * 10.0;
            self.stats.actual_fps_x10.store(fps.round() as u32, Ordering::Relaxed);
            *fps_lock = (Instant::now(), 0);
        }
    }
}

/// Generates a standard 44-byte WAV header for streaming PCM audio.
pub fn generate_wav_header(sample_rate: u32, channels: u16, bits_per_sample: u16) -> [u8; 44] {
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
    let block_align = channels * (bits_per_sample / 8);
    let mut header = [0u8; 44];
    // "RIFF"
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&0x7FFFFFFF_u32.to_le_bytes()); // Maximum length for continuous stream
    header[8..12].copy_from_slice(b"WAVE");
    // "fmt "
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes()); // Subchunk1Size for PCM
    header[20..22].copy_from_slice(&1u16.to_le_bytes());  // AudioFormat = PCM (1)
    header[22..24].copy_from_slice(&channels.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&bits_per_sample.to_le_bytes());
    // "data"
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&0x7FFFFFFF_u32.to_le_bytes()); // Subchunk2Size
    header
}

#[derive(Clone)]
pub struct AudioBroadcaster {
    sender: broadcast::Sender<bytes::Bytes>,
    audio_level: Arc<AtomicU8>,
    viewer_count: Arc<AtomicUsize>,
}

impl Default for AudioBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBroadcaster {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(128);
        Self {
            sender,
            audio_level: Arc::new(AtomicU8::new(0)),
            viewer_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<bytes::Bytes> {
        self.sender.subscribe()
    }

    pub fn broadcast_audio(&self, data: &[u8], level: u8) {
        self.audio_level.store(level, Ordering::Relaxed);
        if self.sender.receiver_count() > 0 && !data.is_empty() {
            let chunk = bytes::Bytes::copy_from_slice(data);
            let _ = self.sender.send(chunk);
        }
    }

    pub fn get_audio_level(&self) -> u8 {
        self.audio_level.load(Ordering::Relaxed)
    }

    pub fn increment_viewer(&self) {
        self.viewer_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_viewer(&self) {
        self.viewer_count.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn get_viewer_count(&self) -> usize {
        self.viewer_count.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_broadcaster_lifecycle() {
        let broadcaster = AudioBroadcaster::new();
        assert_eq!(broadcaster.get_viewer_count(), 0);
        assert_eq!(broadcaster.get_audio_level(), 0);

        broadcaster.increment_viewer();
        assert_eq!(broadcaster.get_viewer_count(), 1);

        let mut rx = broadcaster.subscribe();
        let test_pcm = vec![12u8, 34, 56, 78];
        broadcaster.broadcast_audio(&test_pcm, 42);

        assert_eq!(broadcaster.get_audio_level(), 42);
        let received = rx.try_recv().expect("Should receive audio chunk");
        assert_eq!(received.as_ref(), &[12u8, 34, 56, 78]);

        broadcaster.decrement_viewer();
        assert_eq!(broadcaster.get_viewer_count(), 0);
    }

    #[test]
    fn test_generate_wav_header() {
        let header = generate_wav_header(16000, 1, 16);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(&header[12..16], b"fmt ");
        assert_eq!(&header[36..40], b"data");
        // Sample rate: 16000
        let sr = u32::from_le_bytes(header[24..28].try_into().unwrap());
        assert_eq!(sr, 16000);
        // Channels: 1
        let ch = u16::from_le_bytes(header[22..24].try_into().unwrap());
        assert_eq!(ch, 1);
        // Bits per sample: 16
        let bits = u16::from_le_bytes(header[34..36].try_into().unwrap());
        assert_eq!(bits, 16);
    }

    #[test]
    fn test_broadcaster_viewer_count() {
        let broadcaster = FrameBroadcaster::new();
        assert_eq!(broadcaster.get_viewer_count(), 0);

        broadcaster.increment_viewer();
        broadcaster.increment_viewer();
        assert_eq!(broadcaster.get_viewer_count(), 2);

        broadcaster.decrement_viewer();
        assert_eq!(broadcaster.get_viewer_count(), 1);
    }

    #[test]
    fn test_broadcast_and_receive() {
        let broadcaster = FrameBroadcaster::new();
        let rx = broadcaster.subscribe();

        assert!(rx.borrow().is_none());

        broadcaster.broadcast_frame(vec![0xFF, 0xD8, 0xFF, 0xE0], 640, 480, 123456);

        let latest = broadcaster.get_latest_frame().unwrap();
        assert_eq!(latest.width, 640);
        assert_eq!(latest.height, 480);
        assert_eq!(latest.jpeg_data.as_slice(), &[0xFF, 0xD8, 0xFF, 0xE0]);

        assert!(rx.borrow().is_some());
        let received = rx.borrow().as_ref().unwrap().clone();
        assert_eq!(received.frame_seq, 1);
    }
}
