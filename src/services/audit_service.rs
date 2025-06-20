use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::Utc;
use tracing::info;

/// Audit logging service for tracking user actions
pub struct AuditService;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditAction {
    // Authentication
    UserLogin,
    UserLogout,
    UserRegister,
    LoginFailed,
    
    // Profile
    ProfileUpdate,
    PasswordChange,
    ColorChange,
    
    // Chat
    MessageSent,
    MessageDeleted,
    ChannelJoined,
    ChannelLeft,
    DMSent,
    
    // Forum
    ThreadCreated,
    PostCreated,
    PostDeleted,
    ThreadDeleted,
    
    // Moderation
    UserBanned,
    UserUnbanned,
    UserWarned,
    MessageModerated,
    
    // Server
    ServerInviteSent,
    ServerInviteAccepted,
    ServerInviteDeclined,
    
    // Admin
    ConfigurationChanged,
    UserRoleChanged,
    ChannelCreated,
    ChannelDeleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub timestamp: i64,
    pub action: AuditAction,
    pub user_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    pub target_id: Option<Uuid>, // Channel, thread, post, etc.
    pub ip_address: Option<String>,
    pub metadata: HashMap<String, String>,
    pub details: Option<String>,
}

impl AuditService {
    /// Log a user action
    pub async fn log_action(
        action: AuditAction,
        user_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        target_id: Option<Uuid>,
        ip_address: Option<String>,
        metadata: HashMap<String, String>,
        details: Option<String>,
    ) -> Result<(), String> {
        let entry = AuditEntry {
            id: Uuid::new_v4(),
            timestamp: Utc::now().timestamp(),
            action: action.clone(),
            user_id,
            target_user_id,
            target_id,
            ip_address,
            metadata,
            details,
        };
        
        // Store in database
        Self::store_audit_entry(&entry).await?;
        
        info!("Audit log: {:?} by user {:?}", action, user_id);
        Ok(())
    }
    
    /// Log authentication events
    pub async fn log_auth_event(
        action: AuditAction,
        username: &str,
        ip_address: Option<String>,
        success: bool,
    ) {
        let mut metadata = HashMap::new();
        metadata.insert("username".to_string(), username.to_string());
        metadata.insert("success".to_string(), success.to_string());
        
        let details = if success {
            Some(format!("Successful {} for user: {}", 
                match action {
                    AuditAction::UserLogin => "login",
                    AuditAction::UserRegister => "registration",
                    _ => "authentication"
                }, username))
        } else {
            Some(format!("Failed {} attempt for user: {}", 
                match action {
                    AuditAction::LoginFailed => "login",
                    _ => "authentication"
                }, username))
        };
        
        if let Err(e) = Self::log_action(
            action, None, None, None, ip_address, metadata, details
        ).await {
            tracing::error!("Failed to log auth event: {}", e);
        }
    }
    
    /// Log message actions
    pub async fn log_message_action(
        action: AuditAction,
        user_id: Uuid,
        message_id: Option<Uuid>,
        channel_id: Option<Uuid>,
        content_preview: Option<&str>,
    ) {
        let mut metadata = HashMap::new();
        
        if let Some(channel_id) = channel_id {
            metadata.insert("channel_id".to_string(), channel_id.to_string());
        }
        
        if let Some(message_id) = message_id {
            metadata.insert("message_id".to_string(), message_id.to_string());
        }
        
        let details = content_preview.map(|content| {
            let preview = if content.len() > 100 {
                format!("{}...", &content[..100])
            } else {
                content.to_string()
            };
            format!("Message content preview: {}", preview)
        });
        
        if let Err(e) = Self::log_action(
            action, Some(user_id), None, message_id, None, metadata, details
        ).await {
            tracing::error!("Failed to log message action: {}", e);
        }
    }
    
    /// Log moderation actions
    pub async fn log_moderation_action(
        action: AuditAction,
        moderator_id: Uuid,
        target_user_id: Option<Uuid>,
        reason: Option<&str>,
        duration: Option<&str>,
    ) {
        let mut metadata = HashMap::new();
        
        if let Some(reason) = reason {
            metadata.insert("reason".to_string(), reason.to_string());
        }
        
        if let Some(duration) = duration {
            metadata.insert("duration".to_string(), duration.to_string());
        }
        
        let details = match action {
            AuditAction::UserBanned => Some("User banned by moderator".to_string()),
            AuditAction::UserUnbanned => Some("User unbanned by moderator".to_string()),
            AuditAction::UserWarned => Some("User warned by moderator".to_string()),
            AuditAction::MessageModerated => Some("Message moderated".to_string()),
            _ => None,
        };
        
        if let Err(e) = Self::log_action(
            action, Some(moderator_id), target_user_id, None, None, metadata, details
        ).await {
            tracing::error!("Failed to log moderation action: {}", e);
        }
    }
    
    /// Get audit logs with pagination
    pub async fn get_audit_logs(
        limit: usize,
        offset: usize,
        user_filter: Option<Uuid>,
        action_filter: Option<AuditAction>,
        start_time: Option<i64>,
        end_time: Option<i64>,
    ) -> Result<Vec<AuditEntry>, String> {
        Self::fetch_audit_entries(limit, offset, user_filter, action_filter, start_time, end_time).await
    }
    
    /// Get audit statistics
    pub async fn get_audit_stats(
        start_time: Option<i64>,
        end_time: Option<i64>,
    ) -> Result<AuditStats, String> {
        Self::calculate_audit_stats(start_time, end_time).await
    }
    
    // Database operations (would be implemented in db module)
    async fn store_audit_entry(entry: &AuditEntry) -> Result<(), String> {
        // This would typically store in database
        // For now, we'll implement a simple file-based storage
        use std::fs::OpenOptions;
        use std::io::Write;
        
        let log_line = format!(
            "{},{:?},{},{},{},{},{}\n",
            entry.timestamp,
            entry.action,
            entry.user_id.map(|u| u.to_string()).unwrap_or_else(|| "None".to_string()),
            entry.target_user_id.map(|u| u.to_string()).unwrap_or_else(|| "None".to_string()),
            entry.target_id.map(|u| u.to_string()).unwrap_or_else(|| "None".to_string()),
            entry.ip_address.as_deref().unwrap_or("None"),
            entry.details.as_deref().unwrap_or("")
        );
        
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open("audit.log")
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(log_line.as_bytes()) {
                    return Err(format!("Failed to write audit log: {}", e));
                }
            }
            Err(e) => return Err(format!("Failed to open audit log file: {}", e)),
        }
        
        Ok(())
    }
    
    async fn fetch_audit_entries(
        _limit: usize,
        _offset: usize,
        _user_filter: Option<Uuid>,
        _action_filter: Option<AuditAction>,
        _start_time: Option<i64>,
        _end_time: Option<i64>,
    ) -> Result<Vec<AuditEntry>, String> {
        // Placeholder implementation
        // In a real implementation, this would query the database
        Ok(Vec::new())
    }
    
    async fn calculate_audit_stats(
        _start_time: Option<i64>,
        _end_time: Option<i64>,
    ) -> Result<AuditStats, String> {
        // Placeholder implementation
        Ok(AuditStats {
            total_entries: 0,
            unique_users: 0,
            actions_by_type: HashMap::new(),
            most_active_users: Vec::new(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditStats {
    pub total_entries: usize,
    pub unique_users: usize,
    pub actions_by_type: HashMap<String, usize>,
    pub most_active_users: Vec<(Uuid, usize)>,
}