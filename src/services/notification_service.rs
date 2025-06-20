// server/src/services/notification_service.rs

use crate::db::notifications;
use crate::errors::{Result, ServerError};
use crate::services::BroadcastService;
use crate::api::connection::PeerMap;
use common::{Notification, ServerMessage};
use tracing::{error, info};
use uuid::Uuid;

pub struct NotificationService;

impl NotificationService {
    /// Create a DM notification
    pub async fn create_dm_notification(
        user_id: Uuid,
        dm_id: Uuid,
        from_username: &str,
        peer_map: &PeerMap,
    ) {
        let extra = format!("From: {}", from_username);
        
        if let Err(e) = notifications::db_insert_notification(
            user_id,
            "DM",
            dm_id,
            Some(extra),
        ).await {
            error!("Failed to create DM notification: {}", e);
            return;
        }

        // Push notification if user is online
        Self::push_notifications_if_online(peer_map, user_id).await;
        
        info!("DM notification created for user {}", user_id);
    }

    /// Create a mention notification
    pub async fn create_mention_notification(
        user_id: Uuid,
        from_user_id: Uuid,
        content: &str,
        peer_map: &PeerMap,
    ) {
        let extra = format!("Message: {}", content);
        
        if let Err(e) = notifications::db_insert_notification(
            user_id,
            "Mention",
            from_user_id,
            Some(extra),
        ).await {
            error!("Failed to create mention notification: {}", e);
            return;
        }

        // Push notification if user is online
        Self::push_notifications_if_online(peer_map, user_id).await;
        
        info!("Mention notification created for user {}", user_id);
    }

    /// Get user notifications with pagination
    pub async fn get_notifications(
        user_id: Uuid,
        before: Option<i64>,
    ) -> Result<(Vec<Notification>, bool)> {
        notifications::db_get_notifications(user_id, before).await
            .map_err(|e| ServerError::Database(e))
    }

    /// Mark notification as read
    pub async fn mark_notification_read(notification_id: Uuid) -> Result<()> {
        notifications::db_mark_notification_read(notification_id).await
            .map_err(|e| ServerError::Database(e))?;
        
        info!("Notification {} marked as read", notification_id);
        Ok(())
    }

    /// Push notifications to user if they're online
    async fn push_notifications_if_online(peer_map: &PeerMap, user_id: Uuid) {
        if BroadcastService::is_user_online(peer_map, user_id).await {
            if let Ok((notifications, history_complete)) = 
                notifications::db_get_notifications(user_id, None).await 
            {
                let message = ServerMessage::Notifications { 
                    notifications, 
                    history_complete 
                };
                
                BroadcastService::send_to_user(peer_map, user_id, &message).await;
            }
        }
    }
}