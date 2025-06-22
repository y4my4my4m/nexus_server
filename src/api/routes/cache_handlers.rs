use super::MessageRouter;
use crate::services::BroadcastService;
use common::ServerMessage;
use tokio::sync::mpsc;

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
}