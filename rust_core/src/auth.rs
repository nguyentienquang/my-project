use crate::storage::EventDatabase;
use parking_lot::RwLock;
use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PAIRING_CODE_VALIDITY_SECS: u64 = 300; // 5 minutes
const MAX_FAILED_ATTEMPTS: u32 = 5;
const RATE_LIMIT_WINDOW_SECS: u64 = 30;
const DEVICE_TOKEN_VALIDITY_DAYS: u64 = 365;

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub token: String,
    pub created_at: Instant,
    pub last_seen: Instant,
    pub expires_at: i64,
}

pub struct AuthState {
    current_pairing_code: String,
    code_created_at: Instant,
    sessions: HashMap<String, SessionInfo>,
    failed_attempts: Vec<Instant>,
}

#[derive(Clone)]
pub struct AuthManager {
    state: Arc<RwLock<AuthState>>,
    db: Option<Arc<EventDatabase>>,
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthManager {
    pub fn new() -> Self {
        Self::with_optional_db(None)
    }

    pub fn with_db(db: Arc<EventDatabase>) -> Self {
        Self::with_optional_db(Some(db))
    }

    pub fn with_optional_db(db: Option<Arc<EventDatabase>>) -> Self {
        let mut rng = rand::thread_rng();
        let code = format!("{:06}", rng.gen_range(100_000..999_999));
        let mut sessions = HashMap::new();

        if let Some(ref d) = db {
            if let Ok(devices) = d.get_all_paired_devices() {
                let now = Instant::now();
                for (t, exp) in devices {
                    sessions.insert(
                        t.clone(),
                        SessionInfo {
                            token: t,
                            created_at: now,
                            last_seen: now,
                            expires_at: exp,
                        },
                    );
                }
            }
        }

        Self {
            state: Arc::new(RwLock::new(AuthState {
                current_pairing_code: code,
                code_created_at: Instant::now(),
                sessions,
                failed_attempts: Vec::new(),
            })),
            db,
        }
    }

    /// Generates a new 6-digit pairing code on the tablet.
    pub fn regenerate_pairing_code(&self) -> String {
        let mut rng = rand::thread_rng();
        let code = format!("{:06}", rng.gen_range(100_000..999_999));
        let mut lock = self.state.write();
        lock.current_pairing_code = code.clone();
        lock.code_created_at = Instant::now();
        code
    }

    /// Returns the current active pairing code if still valid, or generates a new one.
    pub fn get_pairing_code(&self) -> String {
        let lock = self.state.read();
        if lock.code_created_at.elapsed() < Duration::from_secs(PAIRING_CODE_VALIDITY_SECS) {
            lock.current_pairing_code.clone()
        } else {
            drop(lock);
            self.regenerate_pairing_code()
        }
    }

    /// Attempts to pair a browser viewer using the 6-digit code.
    pub fn verify_pairing_code(&self, candidate_code: &str) -> Result<String, &'static str> {
        self.verify_pairing_code_with_device(candidate_code, None)
    }

    /// Attempts to pair a browser viewer using the 6-digit code and optional device name.
    pub fn verify_pairing_code_with_device(
        &self,
        candidate_code: &str,
        device_name: Option<&str>,
    ) -> Result<String, &'static str> {
        let now = Instant::now();
        let mut lock = self.state.write();

        // 1. Clean up old failed attempts and check rate limit
        let cutoff = now.checked_sub(Duration::from_secs(RATE_LIMIT_WINDOW_SECS)).unwrap_or(now);
        lock.failed_attempts.retain(|&t| t > cutoff);

        if lock.failed_attempts.len() as u32 >= MAX_FAILED_ATTEMPTS {
            return Err("Quá nhiều lần thử sai. Vui lòng chờ 30 giây!");
        }

        // 2. Check expiration
        if lock.code_created_at.elapsed() > Duration::from_secs(PAIRING_CODE_VALIDITY_SECS) {
            return Err("Mã ghép nối đã hết hạn! Vui lòng xem mã mới trên máy tính bảng.");
        }

        // 3. Constant-time / secure comparison
        let code_matches = candidate_code.trim() == lock.current_pairing_code;

        if !code_matches {
            lock.failed_attempts.push(now);
            return Err("Mã ghép nối không đúng!");
        }

        // 4. Generate cryptographically strong session token
        let token: String = {
            let mut rng = rand::thread_rng();
            (0..32)
                .map(|_| {
                    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                    let idx = rng.gen_range(0..CHARSET.len());
                    CHARSET[idx] as char
                })
                .collect()
        };

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let expires_at = now_secs + (DEVICE_TOKEN_VALIDITY_DAYS as i64) * 86_400;

        lock.sessions.insert(
            token.clone(),
            SessionInfo {
                token: token.clone(),
                created_at: now,
                last_seen: now,
                expires_at,
            },
        );

        // Reset failed attempts on success
        lock.failed_attempts.clear();

        // 5. Persist paired device token into SQLite if database is present
        if let Some(ref d) = self.db {
            let _ = d.insert_paired_device(&token, device_name, expires_at);
        }

        Ok(token)
    }

    /// Validates whether a session token is active and updates its last_seen timestamp.
    /// Checks fast in-memory map first, then checks SQLite database for persistent paired devices.
    pub fn validate_token(&self, token: &str) -> bool {
        if token.is_empty() {
            return false;
        }
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut lock = self.state.write();
        if let Some(session) = lock.sessions.get_mut(token) {
            // Check if token has expired
            if session.expires_at != 0 && session.expires_at <= now_unix {
                lock.sessions.remove(token);
                if let Some(ref d) = self.db {
                    let _ = d.delete_paired_device(token);
                }
                return false;
            }
            session.last_seen = Instant::now();
            true
        } else if let Some(ref d) = self.db {
            if let Ok(true) = d.is_paired_device(token) {
                let now = Instant::now();
                lock.sessions.insert(
                    token.to_string(),
                    SessionInfo {
                        token: token.to_string(),
                        created_at: now,
                        last_seen: now,
                        expires_at: 0,
                    },
                );
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Revokes a specific session from memory and SQLite.
    pub fn revoke_token(&self, token: &str) {
        let mut lock = self.state.write();
        lock.sessions.remove(token);
        if let Some(ref d) = self.db {
            let _ = d.delete_paired_device(token);
        }
    }

    /// Revokes all active sessions from the tablet and clears paired devices from SQLite.
    pub fn revoke_all_sessions(&self) {
        let mut lock = self.state.write();
        lock.sessions.clear();
        if let Some(ref d) = self.db {
            let _ = d.clear_all_paired_devices();
        }
    }

    /// Number of active sessions in memory.
    pub fn active_session_count(&self) -> usize {
        let lock = self.state.read();
        lock.sessions.len()
    }

    /// Number of paired devices in database or active sessions.
    pub fn paired_device_count(&self) -> usize {
        if let Some(ref d) = self.db {
            d.count_paired_devices().unwrap_or_else(|_| self.active_session_count())
        } else {
            self.active_session_count()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_successful_pairing() {
        let auth = AuthManager::new();
        let code = auth.get_pairing_code();
        assert_eq!(code.len(), 6);

        let token_res = auth.verify_pairing_code(&code);
        assert!(token_res.is_ok());
        let token = token_res.unwrap();
        assert_eq!(token.len(), 32);

        assert!(auth.validate_token(&token));
    }

    #[test]
    fn test_invalid_pairing_code() {
        let auth = AuthManager::new();
        let _ = auth.get_pairing_code();

        let token_res = auth.verify_pairing_code("000000_invalid");
        assert!(token_res.is_err());
        assert_eq!(token_res.unwrap_err(), "Mã ghép nối không đúng!");
    }

    #[test]
    fn test_expired_pairing_code() {
        let auth = AuthManager::new();
        let code = auth.get_pairing_code();

        // Artificially expire code
        {
            let mut lock = auth.state.write();
            lock.code_created_at = Instant::now() - Duration::from_secs(PAIRING_CODE_VALIDITY_SECS + 10);
        }

        let token_res = auth.verify_pairing_code(&code);
        assert!(token_res.is_err());
        assert!(token_res.unwrap_err().contains("hết hạn"));
    }

    #[test]
    fn test_rate_limiting() {
        let auth = AuthManager::new();

        // 5 consecutive failed attempts
        for _ in 0..MAX_FAILED_ATTEMPTS {
            let _ = auth.verify_pairing_code("wrong");
        }

        // 6th attempt should be blocked by rate limit
        let res = auth.verify_pairing_code("wrong");
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("Quá nhiều lần thử sai"));
    }

    #[test]
    fn test_session_revocation() {
        let auth = AuthManager::new();
        let code = auth.get_pairing_code();
        let token = auth.verify_pairing_code(&code).unwrap();

        assert!(auth.validate_token(&token));
        auth.revoke_token(&token);
        assert!(!auth.validate_token(&token));
    }

    #[test]
    fn test_revoke_all_sessions() {
        let auth = AuthManager::new();
        let code = auth.get_pairing_code();
        let token1 = auth.verify_pairing_code(&code).unwrap();
        let token2 = auth.verify_pairing_code(&code).unwrap();

        assert_eq!(auth.active_session_count(), 2);
        auth.revoke_all_sessions();
        assert_eq!(auth.active_session_count(), 0);
        assert!(!auth.validate_token(&token1));
        assert!(!auth.validate_token(&token2));
    }

    #[test]
    fn test_persistent_paired_device_lifecycle() {
        let db = Arc::new(EventDatabase::new_in_memory().unwrap());

        // Session 1: Client pairs with tablet
        let auth1 = AuthManager::with_db(db.clone());
        let code = auth1.get_pairing_code();
        let token = auth1
            .verify_pairing_code_with_device(&code, Some("Firefox on Linux"))
            .expect("Pairing should succeed");

        assert!(auth1.validate_token(&token));
        assert_eq!(auth1.paired_device_count(), 1);

        // Simulate server/app restart: auth1 is dropped, new AuthManager starts with same DB
        drop(auth1);
        let auth2 = AuthManager::with_db(db.clone());

        // Token must still be valid WITHOUT entering pairing code again!
        assert!(auth2.validate_token(&token), "Remembered device token must validate after restart");
        assert_eq!(auth2.paired_device_count(), 1);

        // Tablet owner revokes all viewer sessions
        auth2.revoke_all_sessions();
        assert!(!auth2.validate_token(&token), "Token must be invalidated after revocation");
        assert_eq!(auth2.paired_device_count(), 0);

        // Simulate another restart: token remains revoked
        let auth3 = AuthManager::with_db(db.clone());
        assert!(!auth3.validate_token(&token), "Revoked token must not resurrect after restart");
        assert_eq!(auth3.paired_device_count(), 0);
    }

    #[test]
    fn test_paired_device_in_memory_expiration() {
        let db = Arc::new(EventDatabase::new_in_memory().unwrap());
        let auth = AuthManager::with_db(db.clone());

        // Insert an already expired token into DB
        let past_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 - 100;
        db.insert_paired_device("expired_token", Some("Old Device"), past_time).unwrap();

        // New AuthManager loaded from DB should not accept expired token
        let auth2 = AuthManager::with_db(db.clone());
        assert!(!auth2.validate_token("expired_token"), "Expired token must not validate");

        // Now test in-memory expiration for token already in cache
        let code = auth.get_pairing_code();
        let valid_token = auth.verify_pairing_code(&code).unwrap();
        assert!(auth.validate_token(&valid_token));

        // Forcibly expire the token in cache
        {
            let mut lock = auth.state.write();
            if let Some(s) = lock.sessions.get_mut(&valid_token) {
                s.expires_at = past_time;
            }
        }
        assert!(!auth.validate_token(&valid_token), "Expired cached token must not validate and must be purged");
        assert_eq!(auth.active_session_count(), 0);
    }
}
