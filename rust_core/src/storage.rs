use crate::motion::MotionEvent;
use parking_lot::Mutex;
use rusqlite::{params, Connection, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct EventDatabase {
    conn: Arc<Mutex<Connection>>,
    pub storage_dir: PathBuf,
}

impl EventDatabase {
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>>(db_path: P, storage_dir: Q) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            storage_dir: storage_dir.as_ref().to_path_buf(),
        };
        db.init_schema()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let temp_dir = std::env::temp_dir().join(format!("watchcam_test_{}", rand::random::<u32>()));
        let _ = fs::create_dir_all(&temp_dir);
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            storage_dir: temp_dir,
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS motion_events (
                id TEXT PRIMARY KEY,
                timestamp INTEGER NOT NULL,
                motion_score REAL NOT NULL,
                camera_id TEXT NOT NULL,
                snapshot_path TEXT NOT NULL,
                event_type TEXT NOT NULL,
                region_json TEXT,
                video_path TEXT
            )",
            [],
        )?;

        // Ensure video_path column exists for migrations
        let _ = conn.execute("ALTER TABLE motion_events ADD COLUMN video_path TEXT", []);

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_motion_events_timestamp ON motion_events (timestamp DESC)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS paired_devices (
                token TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                last_seen INTEGER NOT NULL,
                device_name TEXT,
                expires_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_paired_devices_token ON paired_devices (token)",
            [],
        )?;
        Ok(())
    }

    pub fn insert_event(&self, event: &MotionEvent) -> Result<()> {
        let conn = self.conn.lock();
        let region_json = if event.region.is_some() || (!event.entity_type.is_empty() && event.entity_type != "motion") || event.confidence > 0.0 {
            Some(serde_json::json!({
                "box": event.region,
                "entity": if event.entity_type.is_empty() { "motion" } else { &event.entity_type },
                "confidence": event.confidence,
            }).to_string())
        } else {
            event.region.as_ref().and_then(|r| serde_json::to_string(r).ok())
        };

        conn.execute(
            "INSERT INTO motion_events (id, timestamp, motion_score, camera_id, snapshot_path, event_type, region_json, video_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.id,
                event.timestamp as i64,
                event.motion_score,
                event.camera_id,
                event.snapshot_path,
                event.event_type,
                region_json,
                event.video_path,
            ],
        )?;
        Ok(())
    }

    pub fn update_event_video_path(&self, id: &str, video_path: &str) -> Result<()> {
        let conn = self.conn.lock();
        let updated = conn.execute(
            "UPDATE motion_events SET video_path = ?1 WHERE id = ?2",
            params![video_path, id],
        )?;
        if updated == 0 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let _ = conn.execute(
                "INSERT INTO motion_events (id, timestamp, motion_score, camera_id, snapshot_path, event_type, region_json, video_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, now, 0.95f32, "camera", "", "motion_detected", Option::<String>::None, video_path],
            );
        }
        Ok(())
    }

    pub fn get_events(&self, limit: usize, offset: usize) -> Result<Vec<MotionEvent>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, motion_score, camera_id, snapshot_path, event_type, region_json, video_path
             FROM motion_events
             ORDER BY timestamp DESC
             LIMIT ?1 OFFSET ?2",
        )?;

        let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
            let id: String = row.get(0)?;
            let ts: i64 = row.get(1)?;
            let motion_score: f32 = row.get(2)?;
            let camera_id: String = row.get(3)?;
            let snapshot_path: String = row.get(4)?;
            let event_type: String = row.get(5)?;
            let region_json: Option<String> = row.get(6)?;
            let video_path: Option<String> = row.get(7)?;

            let (region, entity_type, confidence) = if let Some(s) = region_json {
                if let Ok(boxed) = serde_json::from_str::<[f32; 4]>(&s) {
                    (Some(boxed), "motion".to_string(), motion_score)
                } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&s) {
                    let bbox = val.get("box").and_then(|b| serde_json::from_value::<[f32; 4]>(b.clone()).ok());
                    let ent = val.get("entity").and_then(|e| e.as_str()).unwrap_or("motion").to_string();
                    let conf = val.get("confidence").and_then(|c| c.as_f64()).map(|c| c as f32).unwrap_or(motion_score);
                    (bbox, ent, conf)
                } else {
                    (None, "motion".to_string(), motion_score)
                }
            } else {
                (None, "motion".to_string(), motion_score)
            };

            let has_video = video_path.as_ref().map(|p| !p.is_empty()).unwrap_or(false)
                || Path::new(&snapshot_path).parent().unwrap_or(Path::new(".")).join(format!("motion_{}.mp4", id)).exists()
                || self.storage_dir.join(format!("motion_{}.mp4", id)).exists();

            Ok(MotionEvent {
                id,
                timestamp: ts as u64,
                motion_score,
                camera_id,
                snapshot_path,
                event_type,
                region,
                video_path,
                has_video,
                entity_type,
                confidence,
            })
        })?;

        let mut events = Vec::new();
        for r in rows {
            events.push(r?);
        }
        Ok(events)
    }

    pub fn get_latest_event(&self) -> Result<Option<MotionEvent>> {
        let events = self.get_events(1, 0)?;
        Ok(events.into_iter().next())
    }

    pub fn get_event(&self, id: &str) -> Result<Option<MotionEvent>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, motion_score, camera_id, snapshot_path, event_type, region_json, video_path
             FROM motion_events
             WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            let id: String = row.get(0)?;
            let ts: i64 = row.get(1)?;
            let motion_score: f32 = row.get(2)?;
            let camera_id: String = row.get(3)?;
            let snapshot_path: String = row.get(4)?;
            let event_type: String = row.get(5)?;
            let region_json: Option<String> = row.get(6)?;
            let video_path: Option<String> = row.get(7)?;

            let (region, entity_type, confidence) = if let Some(s) = region_json {
                if let Ok(boxed) = serde_json::from_str::<[f32; 4]>(&s) {
                    (Some(boxed), "motion".to_string(), motion_score)
                } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&s) {
                    let bbox = val.get("box").and_then(|b| serde_json::from_value::<[f32; 4]>(b.clone()).ok());
                    let ent = val.get("entity").and_then(|e| e.as_str()).unwrap_or("motion").to_string();
                    let conf = val.get("confidence").and_then(|c| c.as_f64()).map(|c| c as f32).unwrap_or(motion_score);
                    (bbox, ent, conf)
                } else {
                    (None, "motion".to_string(), motion_score)
                }
            } else {
                (None, "motion".to_string(), motion_score)
            };

            let has_video = video_path.as_ref().map(|p| !p.is_empty()).unwrap_or(false)
                || Path::new(&snapshot_path).parent().unwrap_or(Path::new(".")).join(format!("motion_{}.mp4", id)).exists()
                || self.storage_dir.join(format!("motion_{}.mp4", id)).exists();

            Ok(MotionEvent {
                id,
                timestamp: ts as u64,
                motion_score,
                camera_id,
                snapshot_path,
                event_type,
                region,
                video_path,
                has_video,
                entity_type,
                confidence,
            })
        })?;

        if let Some(res) = rows.next() {
            Ok(Some(res?))
        } else {
            Ok(None)
        }
    }

    pub fn delete_event(&self, id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock();
        let info: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT snapshot_path, video_path FROM motion_events WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        conn.execute("DELETE FROM motion_events WHERE id = ?1", params![id])?;

        if let Some((ref path, ref vid)) = info {
            let _ = fs::remove_file(path);
            if let Some(ref v) = vid {
                let _ = fs::remove_file(v);
            }
            let video_path = self.storage_dir.join(format!("motion_{}.mp4", id));
            let _ = fs::remove_file(video_path);
        }
        Ok(info.map(|(p, _)| p))
    }

    pub fn clear_all_events(&self) -> Result<usize> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT snapshot_path, video_path FROM motion_events")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))?;
        for r in rows.flatten() {
            let _ = fs::remove_file(&r.0);
            if let Some(ref vp) = r.1 {
                let _ = fs::remove_file(vp);
            }
        }
        // Also remove any remaining motion MP4 video files in storage_dir
        if let Ok(entries) = fs::read_dir(&self.storage_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "mp4" {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
        let count = conn.execute("DELETE FROM motion_events", [])?;
        Ok(count)
    }

    pub fn count_events(&self) -> Result<usize> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM motion_events", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Enforces retention policies: removes records older than max_days and safely removes old snapshots.
    pub fn enforce_retention(&self, max_days: u32, max_storage_mb: u64) -> Result<usize> {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let max_age_millis = (max_days as u64) * 86_400 * 1000;
        let cutoff_timestamp = now_millis.saturating_sub(max_age_millis);

        let mut deleted_count = 0;

        // 1. Delete events older than cutoff
        {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare("SELECT id, snapshot_path, video_path FROM motion_events WHERE timestamp < ?1")?;
            let old_events: Vec<(String, String, Option<String>)> = stmt
                .query_map(params![cutoff_timestamp as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .filter_map(|r| r.ok())
                .collect();

            for (id, path, vid) in old_events {
                let _ = fs::remove_file(&path);
                if let Some(ref v) = vid {
                    let _ = fs::remove_file(v);
                }
                let video_path = self.storage_dir.join(format!("motion_{}.mp4", id));
                let _ = fs::remove_file(video_path);
                conn.execute("DELETE FROM motion_events WHERE id = ?1", params![id])?;
                deleted_count += 1;
            }
        }

        // 2. Storage size limit check
        let max_bytes = max_storage_mb * 1024 * 1024;
        let mut current_bytes = 0u64;

        if let Ok(entries) = fs::read_dir(&self.storage_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        current_bytes += meta.len();
                    }
                }
            }
        }

        if current_bytes > max_bytes {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare("SELECT id, snapshot_path, video_path FROM motion_events ORDER BY timestamp ASC")?;
            let oldest_events: Vec<(String, String, Option<String>)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .filter_map(|r| r.ok())
                .collect();

            for (id, path, vid) in oldest_events {
                if current_bytes <= max_bytes {
                    break;
                }
                if let Ok(meta) = fs::metadata(&path) {
                    current_bytes = current_bytes.saturating_sub(meta.len());
                }
                let _ = fs::remove_file(&path);
                if let Some(ref v) = vid {
                    if let Ok(vmeta) = fs::metadata(v) {
                        current_bytes = current_bytes.saturating_sub(vmeta.len());
                    }
                    let _ = fs::remove_file(v);
                }
                let video_path = self.storage_dir.join(format!("motion_{}.mp4", id));
                if let Ok(vmeta) = fs::metadata(&video_path) {
                    current_bytes = current_bytes.saturating_sub(vmeta.len());
                }
                let _ = fs::remove_file(video_path);
                conn.execute("DELETE FROM motion_events WHERE id = ?1", params![id])?;
                deleted_count += 1;
            }
        }

        Ok(deleted_count)
    }

    /// Persists or updates a paired device token in SQLite.
    pub fn insert_paired_device(
        &self,
        token: &str,
        device_name: Option<&str>,
        expires_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO paired_devices (token, created_at, last_seen, device_name, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![token, now, now, device_name, expires_at],
        )?;
        Ok(())
    }

    /// Checks if a device token is paired and not expired. Updates last_seen if valid.
    pub fn is_paired_device(&self, token: &str) -> Result<bool> {
        let conn = self.conn.lock();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut stmt = conn.prepare(
            "SELECT 1 FROM paired_devices 
             WHERE token = ?1 AND (expires_at = 0 OR expires_at > ?2)",
        )?;
        let exists = stmt.query_row(params![token, now], |_| Ok(())).is_ok();
        if exists {
            let _ = conn.execute(
                "UPDATE paired_devices SET last_seen = ?1 WHERE token = ?2",
                params![now, token],
            );
        }
        Ok(exists)
    }

    /// Retrieves all active, non-expired paired device tokens from SQLite.
    pub fn get_all_paired_tokens(&self) -> Result<Vec<String>> {
        let devices = self.get_all_paired_devices()?;
        Ok(devices.into_iter().map(|(token, _)| token).collect())
    }

    /// Retrieves all active, non-expired paired device (token, expires_at) pairs from SQLite,
    /// purging any expired records in the process.
    pub fn get_all_paired_devices(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn.lock();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // Auto-purge expired paired devices
        let _ = conn.execute(
            "DELETE FROM paired_devices WHERE expires_at != 0 AND expires_at <= ?1",
            params![now],
        );

        let mut stmt = conn.prepare(
            "SELECT token, expires_at FROM paired_devices 
             WHERE expires_at = 0 OR expires_at > ?1",
        )?;
        let rows = stmt.query_map(params![now], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut devices = Vec::new();
        for r in rows {
            devices.push(r?);
        }
        Ok(devices)
    }

    /// Deletes a specific paired device token from SQLite.
    pub fn delete_paired_device(&self, token: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM paired_devices WHERE token = ?1", params![token])?;
        Ok(())
    }

    /// Deletes all paired devices from SQLite (revokes all paired sessions).
    pub fn clear_all_paired_devices(&self) -> Result<usize> {
        let conn = self.conn.lock();
        let count = conn.execute("DELETE FROM paired_devices", [])?;
        Ok(count)
    }

    /// Returns total number of paired devices recorded.
    pub fn count_paired_devices(&self) -> Result<usize> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM paired_devices", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_insert_and_query() {
        let db = EventDatabase::new_in_memory().unwrap();

        let ev1 = MotionEvent {
            id: "evt_1".to_string(),
            timestamp: 1000,
            motion_score: 0.45,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/snap1.jpg".to_string(),
            event_type: "person_detected".to_string(),
            region: Some([0.1, 0.2, 0.3, 0.4]),
            video_path: None,
            has_video: false,
            entity_type: "person".to_string(),
            confidence: 0.88,
        };

        let ev2 = MotionEvent {
            id: "evt_2".to_string(),
            timestamp: 2000,
            motion_score: 0.82,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/snap2.jpg".to_string(),
            event_type: "motion_detected".to_string(),
            region: None,
            video_path: None,
            has_video: false,
            entity_type: "motion".to_string(),
            confidence: 0.82,
        };

        db.insert_event(&ev1).unwrap();
        db.insert_event(&ev2).unwrap();

        assert_eq!(db.count_events().unwrap(), 2);

        let list = db.get_events(10, 0).unwrap();
        assert_eq!(list.len(), 2);
        // Reverse chronological order: ev2 first
        assert_eq!(list[0].id, "evt_2");
        assert_eq!(list[1].id, "evt_1");
        assert_eq!(list[1].region, Some([0.1, 0.2, 0.3, 0.4]));
    }

    #[test]
    fn test_event_delete() {
        let db = EventDatabase::new_in_memory().unwrap();

        let ev = MotionEvent {
            id: "evt_del".to_string(),
            timestamp: 1500,
            motion_score: 0.60,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/snap_del.jpg".to_string(),
            event_type: "motion_detected".to_string(),
            region: None,
            video_path: None,
            has_video: false,
            entity_type: "motion".to_string(),
            confidence: 0.60,
        };

        db.insert_event(&ev).unwrap();
        assert_eq!(db.count_events().unwrap(), 1);

        let deleted_path = db.delete_event("evt_del").unwrap();
        assert_eq!(deleted_path, Some("/tmp/snap_del.jpg".to_string()));
        assert_eq!(db.count_events().unwrap(), 0);
    }

    #[test]
    fn test_clear_all_events() {
        let db = EventDatabase::new_in_memory().unwrap();
        for i in 0..5 {
            let ev = MotionEvent {
                id: format!("evt_{}", i),
                timestamp: 1000 + i as u64,
                motion_score: 0.5,
                camera_id: "back".to_string(),
                snapshot_path: format!("/tmp/snap_{}.jpg", i),
                event_type: "motion".to_string(),
                region: None,
                video_path: None,
                has_video: false,
                entity_type: "motion".to_string(),
                confidence: 0.5,
            };
            db.insert_event(&ev).unwrap();
        }
        assert_eq!(db.count_events().unwrap(), 5);
        let count = db.clear_all_events().unwrap();
        assert_eq!(count, 5);
        assert_eq!(db.count_events().unwrap(), 0);
    }

    #[test]
    fn test_enforce_retention_by_age() {
        let db = EventDatabase::new_in_memory().unwrap();
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Old event (10 days old)
        let old_ts = now_millis - 10 * 86_400 * 1000;
        let old_ev = MotionEvent {
            id: "old_evt".to_string(),
            timestamp: old_ts,
            motion_score: 0.7,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/old_snap.jpg".to_string(),
            event_type: "motion_detected".to_string(),
            region: None,
            video_path: None,
            has_video: false,
            entity_type: "motion".to_string(),
            confidence: 0.7,
        };

        // Fresh event (1 hour old)
        let fresh_ts = now_millis - 3600 * 1000;
        let fresh_ev = MotionEvent {
            id: "fresh_evt".to_string(),
            timestamp: fresh_ts,
            motion_score: 0.9,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/fresh_snap.jpg".to_string(),
            event_type: "motion_detected".to_string(),
            region: None,
            video_path: None,
            has_video: false,
            entity_type: "motion".to_string(),
            confidence: 0.9,
        };

        db.insert_event(&old_ev).unwrap();
        db.insert_event(&fresh_ev).unwrap();
        assert_eq!(db.count_events().unwrap(), 2);

        // Retention policy: max 7 days
        let deleted = db.enforce_retention(7, 500).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(db.count_events().unwrap(), 1);

        let remaining = db.get_events(10, 0).unwrap();
        assert_eq!(remaining[0].id, "fresh_evt");
    }

    #[test]
    fn test_motion_event_metadata_serialization() {
        let ev = MotionEvent {
            id: "evt_meta_1".to_string(),
            timestamp: 1730000000000,
            motion_score: 0.75,
            camera_id: "front".to_string(),
            snapshot_path: "/data/snapshots/snap_1.jpg".to_string(),
            event_type: "person_detected".to_string(),
            region: Some([0.1, 0.2, 0.3, 0.4]),
            video_path: Some("/data/snapshots/motion_1.mp4".to_string()),
            has_video: true,
            entity_type: "person".to_string(),
            confidence: 0.88,
        };

        let json_str = serde_json::to_string(&ev).unwrap();
        assert!(json_str.contains("evt_meta_1"));
        assert!(json_str.contains("1730000000000"));
        assert!(json_str.contains("has_video"));
        assert!(json_str.contains("person"));

        let deserialized: MotionEvent = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.id, ev.id);
        assert_eq!(deserialized.timestamp, ev.timestamp);
        assert_eq!(deserialized.camera_id, "front");
        assert_eq!(deserialized.region, Some([0.1, 0.2, 0.3, 0.4]));
        assert_eq!(deserialized.video_path, Some("/data/snapshots/motion_1.mp4".to_string()));
        assert!(deserialized.has_video);
        assert_eq!(deserialized.entity_type, "person");
        assert_eq!(deserialized.confidence, 0.88);
    }

    #[test]
    fn test_get_latest_event() {
        let db = EventDatabase::new_in_memory().unwrap();
        assert!(db.get_latest_event().unwrap().is_none());

        let ev1 = MotionEvent {
            id: "evt_1".to_string(),
            timestamp: 1000,
            motion_score: 0.5,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/1.jpg".to_string(),
            event_type: "motion".to_string(),
            region: None,
            video_path: None,
            has_video: false,
            entity_type: "motion".to_string(),
            confidence: 0.5,
        };
        let ev2 = MotionEvent {
            id: "evt_2".to_string(),
            timestamp: 2000,
            motion_score: 0.8,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/2.jpg".to_string(),
            event_type: "object_detected".to_string(),
            region: None,
            video_path: None,
            has_video: false,
            entity_type: "object".to_string(),
            confidence: 0.75,
        };
        db.insert_event(&ev1).unwrap();
        db.insert_event(&ev2).unwrap();

        let latest = db.get_latest_event().unwrap().expect("Should have latest event");
        assert_eq!(latest.id, "evt_2");
        assert_eq!(latest.entity_type, "object");
    }

    #[test]
    fn test_delete_event_removes_video_and_snapshot() {
        let db = EventDatabase::new_in_memory().unwrap();
        let snap_file = db.storage_dir.join("snap_evt_vid.jpg");
        let video_file = db.storage_dir.join("motion_evt_vid.mp4");
        let _ = fs::write(&snap_file, b"dummy jpeg data");
        let _ = fs::write(&video_file, b"dummy mp4 video data");
        assert!(snap_file.exists());
        assert!(video_file.exists());

        let ev = MotionEvent {
            id: "evt_vid".to_string(),
            timestamp: 5000,
            motion_score: 0.9,
            camera_id: "back".to_string(),
            snapshot_path: snap_file.to_string_lossy().to_string(),
            event_type: "motion_detected".to_string(),
            region: None,
            video_path: Some(video_file.to_string_lossy().to_string()),
            has_video: true,
            entity_type: "motion".to_string(),
            confidence: 0.9,
        };
        db.insert_event(&ev).unwrap();

        db.delete_event("evt_vid").unwrap();
        assert!(!snap_file.exists(), "Snapshot file should be deleted");
        assert!(!video_file.exists(), "Video file should be deleted with event");
    }

    #[test]
    fn test_update_event_video_path_and_has_video() {
        let db = EventDatabase::new_in_memory().unwrap();
        let ev = MotionEvent {
            id: "evt_upd".to_string(),
            timestamp: 6000,
            motion_score: 0.65,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/snap_upd.jpg".to_string(),
            event_type: "motion_detected".to_string(),
            region: None,
            video_path: None,
            has_video: false,
            entity_type: "motion".to_string(),
            confidence: 0.65,
        };
        db.insert_event(&ev).unwrap();

        let fetched = db.get_event("evt_upd").unwrap().unwrap();
        assert_eq!(fetched.video_path, None);
        assert!(!fetched.has_video);

        db.update_event_video_path("evt_upd", "/tmp/motion_evt_upd.mp4").unwrap();

        let updated = db.get_event("evt_upd").unwrap().unwrap();
        assert_eq!(updated.video_path, Some("/tmp/motion_evt_upd.mp4".to_string()));
        assert!(updated.has_video);
    }

    #[test]
    fn test_paired_devices_persistence_lifecycle() {
        let db = EventDatabase::new_in_memory().unwrap();
        assert_eq!(db.count_paired_devices().unwrap(), 0);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let token1 = "token_alpha_12345678901234567890";
        let token2 = "token_beta_98765432109876543210";

        // Insert token1 valid for 365 days
        db.insert_paired_device(token1, Some("Chrome on Windows"), now + 365 * 86_400).unwrap();
        // Insert token2 permanent (expires_at = 0)
        db.insert_paired_device(token2, Some("Safari on iPhone"), 0).unwrap();

        assert_eq!(db.count_paired_devices().unwrap(), 2);
        assert!(db.is_paired_device(token1).unwrap());
        assert!(db.is_paired_device(token2).unwrap());
        assert!(!db.is_paired_device("non_existent_token").unwrap());

        let tokens = db.get_all_paired_tokens().unwrap();
        assert_eq!(tokens.len(), 2);
        assert!(tokens.contains(&token1.to_string()));
        assert!(tokens.contains(&token2.to_string()));

        // Delete single token
        db.delete_paired_device(token1).unwrap();
        assert_eq!(db.count_paired_devices().unwrap(), 1);
        assert!(!db.is_paired_device(token1).unwrap());
        assert!(db.is_paired_device(token2).unwrap());

        // Clear all
        let cleared = db.clear_all_paired_devices().unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(db.count_paired_devices().unwrap(), 0);
        assert!(!db.is_paired_device(token2).unwrap());
    }

    #[test]
    fn test_paired_device_expiration() {
        let db = EventDatabase::new_in_memory().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let expired_token = "expired_token_12345";
        let active_token = "active_token_67890";

        // Expired 1 hour ago
        db.insert_paired_device(expired_token, Some("Old Device"), now - 3600).unwrap();
        // Valid for 1 hour
        db.insert_paired_device(active_token, Some("New Device"), now + 3600).unwrap();

        assert!(!db.is_paired_device(expired_token).unwrap());
        assert!(db.is_paired_device(active_token).unwrap());

        let tokens = db.get_all_paired_tokens().unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], active_token);
    }
}
