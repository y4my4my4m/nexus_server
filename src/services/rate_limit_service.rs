use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use uuid::Uuid;
use tokio::sync::RwLock;
use common::config::RateLimitConfig;

/// Rate limiting service to prevent spam and abuse
pub struct RateLimitService {
    message_limits: RwLock<HashMap<Uuid, MessageRateLimit>>,
    request_limits: RwLock<HashMap<IpAddr, RequestRateLimit>>,
    file_upload_limits: RwLock<HashMap<Uuid, FileUploadRateLimit>>,
    registration_limits: RwLock<HashMap<IpAddr, RegistrationRateLimit>>,
    login_limits: RwLock<HashMap<IpAddr, LoginRateLimit>>,
    config: RateLimitConfig,
}

#[derive(Debug)]
struct MessageRateLimit {
    count: usize,
    window_start: Instant,
}

#[derive(Debug)]
struct RequestRateLimit {
    count: usize,
    window_start: Instant,
}

#[derive(Debug)]
struct FileUploadRateLimit {
    count: usize,
    window_start: Instant,
}

#[derive(Debug)]
struct RegistrationRateLimit {
    count: usize,
    window_start: Instant,
}

#[derive(Debug)]
struct LoginRateLimit {
    count: usize,
    window_start: Instant,
}

impl RateLimitService {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            message_limits: RwLock::new(HashMap::new()),
            request_limits: RwLock::new(HashMap::new()),
            file_upload_limits: RwLock::new(HashMap::new()),
            registration_limits: RwLock::new(HashMap::new()),
            login_limits: RwLock::new(HashMap::new()),
            config,
        }
    }
    
    /// Check if user can send a message
    pub async fn check_message_rate_limit(&self, user_id: Uuid) -> Result<(), String> {
        let mut limits = self.message_limits.write().await;
        let now = Instant::now();
        
        let entry = limits.entry(user_id).or_insert(MessageRateLimit {
            count: 0,
            window_start: now,
        });
        
        // Reset window if minute has passed
        if now.duration_since(entry.window_start) >= Duration::from_secs(60) {
            entry.count = 0;
            entry.window_start = now;
        }
        
        if entry.count >= self.config.messages_per_minute {
            return Err(format!(
                "Rate limit exceeded. Maximum {} messages per minute.",
                self.config.messages_per_minute
            ));
        }
        
        entry.count += 1;
        Ok(())
    }
    
    /// Check if IP can make a request
    pub async fn check_request_rate_limit(&self, ip: IpAddr) -> Result<(), String> {
        let mut limits = self.request_limits.write().await;
        let now = Instant::now();
        
        let entry = limits.entry(ip).or_insert(RequestRateLimit {
            count: 0,
            window_start: now,
        });
        
        // Reset window if second has passed
        if now.duration_since(entry.window_start) >= Duration::from_secs(1) {
            entry.count = 0;
            entry.window_start = now;
        }
        
        if entry.count >= self.config.requests_per_second {
            return Err(format!(
                "Rate limit exceeded. Maximum {} requests per second.",
                self.config.requests_per_second
            ));
        }
        
        entry.count += 1;
        Ok(())
    }
    
    /// Clean up old entries (should be called periodically)
    pub async fn cleanup_old_entries(&self) {
        let now = Instant::now();
        let cleanup_threshold = Duration::from_secs(3600 * 2); // 2 hours
        
        {
            let mut limits = self.message_limits.write().await;
            limits.retain(|_, entry| now.duration_since(entry.window_start) < cleanup_threshold);
        }
        
        {
            let mut limits = self.request_limits.write().await;
            limits.retain(|_, entry| now.duration_since(entry.window_start) < cleanup_threshold);
        }
        
        {
            let mut limits = self.file_upload_limits.write().await;
            limits.retain(|_, entry| now.duration_since(entry.window_start) < cleanup_threshold);
        }
        
        {
            let mut limits = self.registration_limits.write().await;
            limits.retain(|_, entry| now.duration_since(entry.window_start) < cleanup_threshold);
        }
        
        {
            let mut limits = self.login_limits.write().await;
            limits.retain(|_, entry| now.duration_since(entry.window_start) < cleanup_threshold);
        }
    }
}

#[derive(Debug)]
pub struct RateLimitStats {
    pub tracked_users_messages: usize,
    pub tracked_ips_requests: usize,
    pub tracked_users_uploads: usize,
    pub tracked_ips_registrations: usize,
    pub tracked_ips_logins: usize,
}