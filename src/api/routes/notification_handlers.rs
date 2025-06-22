use super::MessageRouter;
use crate::services::NotificationService;
use common::{ServerMessage, User};
use tokio::sync::mpsc;
use uuid::Uuid;

impl MessageRouter {
    /// Handle get notifications
    pub async fn handle_get_notifications(
        &self,
        current_user: &Option<User>,
        before: Option<i64>,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match NotificationService::get_notifications(user.id, before).await {
                Ok((notifications, history_complete)) => {
                    self.send_response(response_sender, ServerMessage::Notifications { 
                        notifications, 
                        history_complete 
                    });
                }
                Err(_) => {
                    self.send_error(response_sender, "Failed to get notifications");
                }
            }
        }
        Ok(())
    }

    /// Handle mark notification as read
    pub async fn handle_mark_notification_read(
        &self,
        notification_id: Uuid,
        _response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        let _ = NotificationService::mark_notification_read(notification_id).await;
        Ok(())
    }
}