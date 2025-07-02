use super::MessageRouter;
use crate::services::InviteService;
use crate::db;
use nexus_tui_common::{ServerMessage, User};
use tokio::sync::mpsc;
use uuid::Uuid;

impl MessageRouter {
    /// Handle send server invite
    pub async fn handle_send_server_invite(
        &self,
        current_user: &Option<User>,
        to_user_id: Uuid,
        server_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match InviteService::send_server_invite(user.id, to_user_id, server_id, &self.peer_map).await {
                Ok(_) => {
                    self.send_success(response_sender, "Server invite sent successfully!");
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to send invite: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Handle respond to server invite
    pub async fn handle_respond_to_server_invite(
        &self,
        current_user: &Option<User>,
        invite_id: Uuid,
        accept: bool,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match InviteService::respond_to_invite(invite_id, user.id, accept, &self.peer_map).await {
                Ok(_) => {
                    let action = if accept { "accepted" } else { "declined" };
                    self.send_success(response_sender, &format!("Server invite {} successfully!", action));
                    
                    // If accepted, refresh the user's server list
                    if accept {
                        let servers = db::servers::db_get_user_servers(user.id).await.unwrap_or_default();
                        self.send_response(response_sender, ServerMessage::Servers(servers));
                    }
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to respond to invite: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Handle accept server invite from user
    pub async fn handle_accept_server_invite_from_user(
        &self,
        current_user: &Option<User>,
        from_user_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match InviteService::respond_to_invite_from_user(from_user_id, user.id, true, &self.peer_map).await {
                Ok(_) => {
                    self.send_success(response_sender, "Server invite accepted!");
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to accept invite: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Handle decline server invite from user
    pub async fn handle_decline_server_invite_from_user(
        &self,
        current_user: &Option<User>,
        from_user_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match InviteService::respond_to_invite_from_user(from_user_id, user.id, false, &self.peer_map).await {
                Ok(_) => {
                    self.send_success(response_sender, "Server invite declined.");
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to decline invite: {}", e));
                }
            }
        }
        Ok(())
    }
}