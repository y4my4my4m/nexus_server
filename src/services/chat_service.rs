use crate::db::{channels, messages};
use crate::errors::{Result, ServerError};
use crate::services::{BroadcastService, NotificationService};
use crate::api::connection::PeerMap;
use common::{ChannelMessage, DirectMessage, ServerMessage, User};
use tracing::{error, info};
use uuid::Uuid;

/// Configuration for pagination
#[derive(Debug, Clone)]
pub struct PaginationConfig {
    pub default_page_size: usize,
    pub max_page_size: usize,
    pub prefetch_threshold: usize, // How many messages to prefetch
}

impl Default for PaginationConfig {
    fn default() -> Self {
        Self {
            default_page_size: 50,
            max_page_size: 100,
            prefetch_threshold: 10,
        }
    }
}

/// Pagination cursor for efficient message fetching
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaginationCursor {
    /// Timestamp-based cursor (more efficient for time-ordered data)
    Timestamp(i64),
    /// Offset-based cursor (fallback)
    Offset(usize),
    /// Start from beginning
    Start,
}

/// Pagination request parameters
#[derive(Debug, Clone)]
pub struct PaginationRequest {
    pub cursor: PaginationCursor,
    pub limit: usize,
    pub direction: PaginationDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaginationDirection {
    Forward,  // Newer messages
    Backward, // Older messages
}

/// Pagination response with metadata
#[derive(Debug, Clone)]
pub struct PaginationResponse<T> {
    pub items: Vec<T>,
    pub has_more: bool,
    pub next_cursor: Option<PaginationCursor>,
    pub prev_cursor: Option<PaginationCursor>,
    pub total_count: Option<usize>, // Optional for performance
}

/// Trait for messages that have timestamps for pagination
pub trait TimestampedMessage {
    fn timestamp(&self) -> i64;
}

impl TimestampedMessage for ChannelMessage {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

impl TimestampedMessage for DirectMessage {
    fn timestamp(&self) -> i64 {
        self.timestamp
    }
}

pub struct ChatService;

impl ChatService {
    /// Calculate pagination cursors based on messages and request direction
    fn calculate_pagination_cursors<T: TimestampedMessage>(
        messages: &[T],
        has_more: bool,
        direction: &PaginationDirection,
    ) -> (Option<PaginationCursor>, Option<PaginationCursor>) {
        let next_cursor = if has_more && !messages.is_empty() {
            match direction {
                PaginationDirection::Backward => {
                    Some(PaginationCursor::Timestamp(messages.first().unwrap().timestamp()))
                }
                PaginationDirection::Forward => {
                    Some(PaginationCursor::Timestamp(messages.last().unwrap().timestamp()))
                }
            }
        } else {
            None
        };
        
        let prev_cursor = if !messages.is_empty() {
            match direction {
                PaginationDirection::Backward => {
                    Some(PaginationCursor::Timestamp(messages.last().unwrap().timestamp()))
                }
                PaginationDirection::Forward => {
                    Some(PaginationCursor::Timestamp(messages.first().unwrap().timestamp()))
                }
            }
        } else {
            None
        };
        
        (next_cursor, prev_cursor)
    }

    /// Create pagination response for start cursor
    fn create_start_pagination_response<T: TimestampedMessage>(
        messages: Vec<T>,
        has_more: bool,
    ) -> PaginationResponse<T> {
        let next_cursor = if has_more && !messages.is_empty() {
            Some(PaginationCursor::Timestamp(messages.first().unwrap().timestamp()))
        } else {
            None
        };
        
        PaginationResponse {
            items: messages,
            has_more,
            next_cursor,
            prev_cursor: None,
            total_count: None,
        }
    }

    /// Create fallback pagination response for offset cursor
    fn create_fallback_pagination_response<T>(
        messages: Vec<T>,
        has_more: bool,
    ) -> PaginationResponse<T> {
        PaginationResponse {
            items: messages,
            has_more,
            next_cursor: None,
            prev_cursor: None,
            total_count: None,
        }
    }

    /// Generic pagination handler for timestamp-based cursors
    async fn handle_timestamp_pagination<T, F, Fut>(
        request: &PaginationRequest,
        limit: usize,
        before_ts: Option<i64>,
        db_fetch: F,
    ) -> Result<PaginationResponse<T>>
    where
        T: TimestampedMessage,
        F: FnOnce(Option<i64>, usize, bool) -> Fut,
        Fut: std::future::Future<Output = std::result::Result<(Vec<T>, bool), String>>,
    {
        let (messages, has_more) = db_fetch(
            before_ts,
            limit,
            request.direction == PaginationDirection::Backward
        ).await.map_err(|e| ServerError::Database(e))?;
        
        let (next_cursor, prev_cursor) = Self::calculate_pagination_cursors(
            &messages, 
            has_more, 
            &request.direction
        );
        
        Ok(PaginationResponse {
            items: messages,
            has_more,
            next_cursor,
            prev_cursor,
            total_count: None,
        })
    }

    /// Send a channel message
    pub async fn send_channel_message(
        channel_id: Uuid,
        user: &User,
        content: &str,
        peer_map: &PeerMap,
    ) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp();
        
        // Store message in database
        let message_id = channels::db_create_channel_message(
            channel_id, user.id, timestamp, content
        ).await.map_err(|e| ServerError::Database(e))?;

        // Create message object
        let channel_msg = ChannelMessage {
            id: message_id,
            channel_id,
            sent_by: user.id,
            timestamp,
            content: content.to_string(),
            author_username: user.username.clone(),
            author_color: user.color.clone(),
            author_profile_pic: user.profile_pic.clone(),
        };

        // Get channel users for broadcasting
        let channel_users = channels::db_get_channel_user_list(channel_id).await
            .map_err(|e| ServerError::Database(e))?;
        
        let user_ids: Vec<Uuid> = channel_users.iter().map(|u| u.id).collect();

        // Broadcast to channel users
        let message = ServerMessage::NewChannelMessage(channel_msg);
        BroadcastService::broadcast_to_channel_users(peer_map, &user_ids, &message).await;

        // Handle mentions
        let mentioned_users = crate::util::extract_mentions(content);
        if !mentioned_users.is_empty() {
            Self::handle_mentions(user, content, &mentioned_users, peer_map).await;
        }

        info!("Channel message sent by {} in channel {}", user.username, channel_id);
        Ok(())
    }

    /// Send a direct message
    pub async fn send_direct_message(
        from_user: &User,
        to_user_id: Uuid,
        content: &str,
        peer_map: &PeerMap,
    ) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp();
        
        // Store DM in database
        let dm_id = messages::db_store_direct_message(
            from_user.id, to_user_id, content, timestamp
        ).await.map_err(|e| ServerError::Database(e))?;

        // Create DM object
        let dm = DirectMessage {
            id: dm_id,
            from: from_user.id,
            to: to_user_id,
            timestamp,
            content: content.to_string(),
            author_username: from_user.username.clone(),
            author_color: from_user.color.clone(),
            author_profile_pic: from_user.profile_pic.clone(),
        };

        // Send to both users
        let message = ServerMessage::DirectMessage(dm.clone());
        let user_ids = vec![from_user.id, to_user_id];
        BroadcastService::broadcast_to_users(peer_map, &user_ids, &message).await;

        // Create notification for recipient
        NotificationService::create_dm_notification(to_user_id, dm_id, &from_user.username, peer_map).await;

        info!("Direct message sent from {} to {}", from_user.username, to_user_id);
        Ok(())
    }

    /// Get channel messages with enhanced pagination
    pub async fn get_channel_messages_paginated(
        channel_id: Uuid,
        request: PaginationRequest,
        config: Option<PaginationConfig>,
    ) -> Result<PaginationResponse<ChannelMessage>> {
        let config = config.unwrap_or_default();
        let limit = request.limit.min(config.max_page_size).max(1);
        
        match request.cursor {
            PaginationCursor::Timestamp(before_ts) => {
                Self::handle_timestamp_pagination(
                    &request,
                    limit,
                    Some(before_ts),
                    |before, lim, reverse| async move {
                        channels::db_get_channel_messages_by_timestamp(channel_id, before, lim, reverse).await
                    }
                ).await
            }
            PaginationCursor::Start => {
                let (messages, has_more) = channels::db_get_channel_messages_by_timestamp(
                    channel_id, 
                    None, 
                    limit,
                    request.direction == PaginationDirection::Backward
                ).await.map_err(|e| ServerError::Database(e))?;
                
                Ok(Self::create_start_pagination_response(messages, has_more))
            }
            PaginationCursor::Offset(_) => {
                // Fallback to existing implementation for compatibility
                let (messages, has_more) = Self::get_channel_messages(channel_id, None, limit).await?;
                Ok(Self::create_fallback_pagination_response(messages, has_more))
            }
        }
    }

    /// Get direct messages with enhanced pagination
    pub async fn get_direct_messages_paginated(
        user1_id: Uuid,
        user2_id: Uuid,
        request: PaginationRequest,
        config: Option<PaginationConfig>,
    ) -> Result<PaginationResponse<DirectMessage>> {
        let config = config.unwrap_or_default();
        let limit = request.limit.min(config.max_page_size).max(1);
        
        match request.cursor {
            PaginationCursor::Timestamp(before_ts) => {
                Self::handle_timestamp_pagination(
                    &request,
                    limit,
                    Some(before_ts),
                    |before, lim, reverse| async move {
                        messages::db_get_direct_messages_by_timestamp(user1_id, user2_id, before, lim, reverse).await
                    }
                ).await
            }
            PaginationCursor::Start => {
                let (messages, has_more) = messages::db_get_direct_messages_by_timestamp(
                    user1_id, 
                    user2_id, 
                    None, 
                    limit,
                    request.direction == PaginationDirection::Backward
                ).await.map_err(|e| ServerError::Database(e))?;
                
                Ok(Self::create_start_pagination_response(messages, has_more))
            }
            PaginationCursor::Offset(_) => {
                // Fallback to existing implementation
                let (messages, has_more) = Self::get_direct_messages(user1_id, user2_id, None, limit).await?;
                Ok(Self::create_fallback_pagination_response(messages, has_more))
            }
        }
    }

    /// Get channel messages with pagination
    pub async fn get_channel_messages(
        channel_id: Uuid,
        before: Option<i64>,
        _limit: usize,
    ) -> Result<(Vec<ChannelMessage>, bool)> {
        channels::db_get_channel_messages(channel_id, before).await
            .map_err(|e| ServerError::Database(e))
    }

    /// Get direct messages between two users
    pub async fn get_direct_messages(
        user1_id: Uuid,
        user2_id: Uuid,
        before: Option<i64>,
        limit: usize,
    ) -> Result<(Vec<DirectMessage>, bool)> {
        messages::db_get_direct_messages(user1_id, user2_id, before, limit).await
            .map_err(|e| ServerError::Database(e))
    }

    /// Get list of users who have DM history with the given user
    pub async fn get_dm_user_list(user_id: Uuid, peer_map: &PeerMap) -> Result<Vec<User>> {
        let mut users = messages::db_get_dm_user_list(user_id).await
            .map_err(|e| ServerError::Database(e))?;

        // Update online status
        for user in &mut users {
            user.status = if BroadcastService::is_user_online(peer_map, user.id).await {
                common::UserStatus::Connected
            } else {
                common::UserStatus::Offline
            };
        }

        Ok(users)
    }

    /// Get channel user list with online status
    pub async fn get_channel_users(channel_id: Uuid, peer_map: &PeerMap) -> Result<Vec<User>> {
        let mut users = channels::db_get_channel_user_list(channel_id).await
            .map_err(|e| ServerError::Database(e))?;

        // Update online status
        for user in &mut users {
            user.status = if BroadcastService::is_user_online(peer_map, user.id).await {
                common::UserStatus::Connected
            } else {
                common::UserStatus::Offline
            };
        }

        Ok(users)
    }

    /// Handle mention notifications
    async fn handle_mentions(
        from_user: &User,
        content: &str,
        mentioned_usernames: &[String],
        peer_map: &PeerMap,
    ) {
        for username in mentioned_usernames {
            // Find the mentioned user
            if let Ok(mentioned_user) = crate::db::users::db_get_user_by_username(username).await {
                // Send mention notification
                let message = ServerMessage::MentionNotification {
                    from: from_user.clone(),
                    content: content.to_string(),
                };

                if BroadcastService::send_to_user(peer_map, mentioned_user.id, &message).await {
                    info!("Mention notification sent to {}", username);
                } else {
                    // User is offline, create persistent notification
                    NotificationService::create_mention_notification(
                        mentioned_user.id,
                        from_user.id,
                        content,
                        peer_map,
                    ).await;
                }
            }
        }
    }
}