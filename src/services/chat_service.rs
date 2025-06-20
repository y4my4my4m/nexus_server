// server/src/services/chat_service.rs

use crate::db::{channels, messages};
use crate::errors::{Result, ServerError};
use crate::services::{BroadcastService, NotificationService};
use crate::api::connection::PeerMap;
use common::{ChannelMessage, DirectMessage, ServerMessage, User};
use tracing::{error, info};
use uuid::Uuid;

pub struct ChatService;

impl ChatService {
    /// Send a channel message
    pub async fn send_channel_message(
        channel_id: Uuid,
        user: &User,
        content: &str,
        peer_map: &PeerMap,
    ) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp();
        
        // Store message in database
        let message_id = messages::db_create_channel_message(
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
            author_color: user.color,
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
        let mentioned_users = Self::extract_mentions(content);
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
            author_color: from_user.color,
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

    /// Get channel messages with pagination
    pub async fn get_channel_messages(
        channel_id: Uuid,
        before: Option<Uuid>,
        limit: usize,
    ) -> Result<(Vec<ChannelMessage>, bool)> {
        messages::db_get_channel_messages(channel_id, before, limit).await
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

    /// Extract usernames mentioned in a message (@username)
    fn extract_mentions(content: &str) -> Vec<String> {
        let mut mentions = Vec::new();
        let re = regex::Regex::new(r"@([a-zA-Z0-9_]+)").unwrap();
        
        for cap in re.captures_iter(content) {
            if let Some(username) = cap.get(1) {
                mentions.push(username.as_str().to_string());
            }
        }
        
        mentions
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