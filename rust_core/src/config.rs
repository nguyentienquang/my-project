use serde::{Deserialize, Serialize};

fn default_resolution() -> String {
    "480p".to_string()
}

fn default_smtp_host() -> String {
    "smtp.gmail.com".to_string()
}

fn default_smtp_port() -> u16 {
    587
}

fn default_https_port() -> u16 {
    8443
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurveillanceConfig {
    pub motion_enabled: bool,
    pub sensitivity: u8,             // 0..100
    pub pixel_diff_threshold: u8,    // 5..120 (derived from sensitivity or overridden)
    pub min_area_ratio: f32,         // 0.001..0.5 (derived from sensitivity or overridden)
    pub min_consecutive_frames: u32, // 1..10
    pub cooldown_seconds: u64,       // 1..60
    pub analysis_fps: u32,           // 1..15
    pub stream_quality: u8,          // 20..95
    pub target_stream_fps: u32,      // 5..30
    pub server_port: u16,            // 1024..65535
    #[serde(default = "default_https_port")]
    pub https_port: u16,             // 1024..65535
    pub max_retention_days: u32,     // 1..90
    pub max_storage_mb: u64,         // 50..10000
    pub camera_id: String,           // "back" or "front"
    #[serde(default = "default_resolution")]
    pub resolution: String,          // "480p" or "720p"
    pub monitoring_region: Option<[f32; 4]>, // [x, y, w, h] in 0.0..1.0
    #[serde(default)]
    pub email_alerts_enabled: bool,
    #[serde(default = "default_smtp_host")]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_username: String,
    #[serde(default)]
    pub smtp_password: String,
    #[serde(default)]
    pub smtp_recipient: String,
}

impl Default for SurveillanceConfig {
    fn default() -> Self {
        let mut cfg = Self {
            motion_enabled: true,
            sensitivity: 70,
            pixel_diff_threshold: 25,
            min_area_ratio: 0.015,
            min_consecutive_frames: 2,
            cooldown_seconds: 5,
            analysis_fps: 5,
            stream_quality: 80,
            target_stream_fps: 20,
            server_port: 8080,
            https_port: 8443,
            max_retention_days: 7,
            max_storage_mb: 500,
            camera_id: "back".to_string(),
            resolution: "480p".to_string(),
            monitoring_region: None,
            email_alerts_enabled: false,
            smtp_host: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            smtp_username: String::new(),
            smtp_password: String::new(),
            smtp_recipient: String::new(),
        };
        cfg.apply_sensitivity(70);
        cfg
    }
}

impl SurveillanceConfig {
    pub fn apply_sensitivity(&mut self, s: u8) {
        let s = s.min(100);
        self.sensitivity = s;
        // Sensitivity 100: diff threshold 12, min area ratio 0.004 (very sensitive)
        // Sensitivity 0: diff threshold 60, min area ratio 0.08 (low sensitivity)
        let factor = (100 - s) as f32 / 100.0;
        self.pixel_diff_threshold = (12.0 + factor * 48.0).round() as u8;
        self.min_area_ratio = 0.004 + factor * 0.076;
    }

    pub fn validate(&mut self) -> Result<(), &'static str> {
        if self.server_port == 0 {
            return Err("Cổng máy chủ không hợp lệ (phải khác 0)");
        }
        if self.https_port == 0 {
            self.https_port = if self.server_port == 8080 { 8443 } else { self.server_port.wrapping_add(363) };
        }
        if self.stream_quality < 10 || self.stream_quality > 100 {
            self.stream_quality = self.stream_quality.clamp(10, 100);
        }
        if self.target_stream_fps == 0 || self.target_stream_fps > 30 {
            self.target_stream_fps = self.target_stream_fps.clamp(1, 30);
        }
        if self.resolution != "720p" && self.resolution != "480p" {
            self.resolution = "480p".to_string();
        }
        if self.analysis_fps == 0 || self.analysis_fps > 30 {
            self.analysis_fps = self.analysis_fps.clamp(1, 30);
        }
        if self.min_consecutive_frames == 0 {
            self.min_consecutive_frames = 1;
        }
        if self.cooldown_seconds == 0 {
            self.cooldown_seconds = 1;
        }
        if self.smtp_port == 0 {
            self.smtp_port = 587;
        }
        if self.smtp_host.trim().is_empty() {
            self.smtp_host = "smtp.gmail.com".to_string();
        }
        if let Some(region) = self.monitoring_region {
            let [x, y, w, h] = region;
            if x < 0.0 || y < 0.0 || w <= 0.0 || h <= 0.0 || (x + w) > 1.01 || (y + h) > 1.01 {
                return Err("Vùng giám sát phải nằm trong khoảng chuẩn hóa 0.0..1.0");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let mut cfg = SurveillanceConfig::default();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.server_port, 8080);
        assert_eq!(cfg.https_port, 8443);
        assert_eq!(cfg.sensitivity, 70);
        assert!(cfg.pixel_diff_threshold < 30);
        assert!(cfg.min_area_ratio < 0.03);
        assert!(!cfg.email_alerts_enabled);
        assert_eq!(cfg.smtp_host, "smtp.gmail.com");
        assert_eq!(cfg.smtp_port, 587);
    }

    #[test]
    fn test_https_port_validation() {
        let mut cfg = SurveillanceConfig::default();
        assert_eq!(cfg.https_port, 8443);
        cfg.https_port = 0;
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.https_port, 8443);

        cfg.server_port = 9000;
        cfg.https_port = 0;
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.https_port, 9363);
    }

    #[test]
    fn test_smtp_validation() {
        let mut cfg = SurveillanceConfig::default();
        cfg.smtp_port = 0;
        cfg.smtp_host = "   ".to_string();
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.smtp_port, 587);
        assert_eq!(cfg.smtp_host, "smtp.gmail.com");
    }

    #[test]
    fn test_sensitivity_mapping() {
        let mut cfg = SurveillanceConfig::default();
        cfg.apply_sensitivity(100);
        assert_eq!(cfg.pixel_diff_threshold, 12);
        assert!((cfg.min_area_ratio - 0.004).abs() < 1e-5);

        cfg.apply_sensitivity(0);
        assert_eq!(cfg.pixel_diff_threshold, 60);
        assert!((cfg.min_area_ratio - 0.080).abs() < 1e-4);
    }

    #[test]
    fn test_config_validation_invalid_port() {
        let mut cfg = SurveillanceConfig::default();
        cfg.server_port = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_config_validation_clamps() {
        let mut cfg = SurveillanceConfig::default();
        cfg.stream_quality = 120;
        cfg.min_consecutive_frames = 0;
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.stream_quality, 100);
        assert_eq!(cfg.min_consecutive_frames, 1);
    }

    #[test]
    fn test_region_validation() {
        let mut cfg = SurveillanceConfig::default();
        cfg.monitoring_region = Some([0.1, 0.1, 0.5, 0.5]);
        assert!(cfg.validate().is_ok());

        cfg.monitoring_region = Some([0.8, 0.8, 0.5, 0.5]); // sum > 1.0
        assert!(cfg.validate().is_err());
    }
}
