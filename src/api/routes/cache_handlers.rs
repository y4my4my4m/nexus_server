use super::MessageRouter;
use crate::services::BroadcastService;
use common::ServerMessage;
use tokio::sync::mpsc;
use uuid::Uuid;

impl MessageRouter {
    /// Handle get cache stats
    pub async fn handle_get_cache_stats(
        &self,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        // This would typically be handled by a cache service
        // For now, return mock data
        let response = ServerMessage::CacheStats {
            total_entries: 0,
            total_size_mb: 0.0,
            hit_ratio: 0.0,
            expired_entries: 0,
        };
        self.send_response(response_sender, response);
        Ok(())
    }

    /// Handle invalidate image cache
    pub async fn handle_invalidate_image_cache(
        &self,
        keys: Vec<String>,
        _response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        // Broadcast cache invalidation to all connected clients
        let response = ServerMessage::ImageCacheInvalidated { keys };
        BroadcastService::broadcast_to_all(&self.peer_map, &response).await;
        Ok(())
    }

    /// Handle get user avatars request
    pub async fn handle_get_user_avatars(
        &self,
        user_ids: Vec<Uuid>,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        // Limit the number of avatars requested to prevent abuse
        let limited_user_ids = if user_ids.len() > 50 {
            user_ids.into_iter().take(50).collect()
        } else {
            user_ids
        };

        let mut avatars = Vec::new();
        for user_id in limited_user_ids {
            match crate::db::users::db_get_user_avatar(user_id).await {
                Ok(profile_pic) => avatars.push((user_id, profile_pic)),
                Err(_) => avatars.push((user_id, None)), // User not found or no avatar
            }
        }

        let _ = response_sender.send(ServerMessage::UserAvatars { avatars });
        Ok(())
    }
}