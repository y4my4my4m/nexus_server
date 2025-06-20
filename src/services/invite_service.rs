use crate::db::invites::*;
use crate::db::servers::{add_user_to_server, db_is_user_in_server};
use crate::errors::{Result, ServerError};
use crate::services::BroadcastService;
use crate::api::connection::PeerMap;
use common::{ServerInvite, ServerInviteStatus, ServerMessage, User};
use tracing::{error, info};
use uuid::Uuid;
use ratatui::style::Color;

pub struct InviteService;

impl InviteService {
    /// Send a server invite to another user
    pub async fn send_server_invite(
        from_user_id: Uuid,
        to_user_id: Uuid,
        server_id: Uuid,
        peer_map: &PeerMap,
    ) -> Result<Uuid> {
        // Check if the sender is a member of the server
        if !db_is_user_in_server(from_user_id, server_id).await? {
            return Err(ServerError::Authorization("You must be a member of this server to invite others".to_string()));
        }

        // Check if the target user is already in the server
        if db_is_user_in_server(to_user_id, server_id).await? {
            return Err(ServerError::BadRequest("User is already in this server".to_string()));
        }

        // Check if there's already a pending invite
        if db_check_existing_invite(from_user_id, to_user_id, server_id).await? {
            return Err(ServerError::BadRequest("Invite already sent to this user".to_string()));
        }

        // Create the invite
        let invite_id = db_create_server_invite(from_user_id, to_user_id, server_id).await?;
        
        // Get the full invite details to send to the recipient
        if let Some(invite) = db_get_invite_by_id(invite_id).await? {
            let message = ServerMessage::ServerInviteReceived(invite.clone());
            
            // Try to send directly to the user if they're online
            if BroadcastService::send_to_user(peer_map, to_user_id, &message).await {
                info!("Server invite sent to online user {}", to_user_id);
            } else {
                info!("Server invite created for offline user {}", to_user_id);
                // The invite is already stored in the database, so when they come online 
                // and request notifications/invites, they'll see it
            }
            
            // Also notify the sender about successful invite creation
            let sender_message = ServerMessage::Notification(
                "Server invite sent successfully!".to_string(), 
                false
            );
            BroadcastService::send_to_user(peer_map, from_user_id, &sender_message).await;
        }
        
        Ok(invite_id)
    }

    /// Respond to a server invite (accept or decline)
    pub async fn respond_to_invite(
        invite_id: Uuid,
        user_id: Uuid,
        accept: bool,
        peer_map: &PeerMap,
    ) -> Result<ServerInvite> {
        // Get the invite
        let invite = db_get_invite_by_id(invite_id).await?
            .ok_or_else(|| ServerError::NotFound("Invite not found".to_string()))?;

        // Verify the user is the recipient
        if invite.to_user_id != user_id {
            return Err(ServerError::Forbidden("Not authorized to respond to this invite".to_string()));
        }

        // Verify the invite is still pending
        if invite.status != ServerInviteStatus::Pending {
            return Err(ServerError::BadRequest("Invite is no longer pending".to_string()));
        }

        let new_status = if accept {
            ServerInviteStatus::Accepted
        } else {
            ServerInviteStatus::Declined
        };

        // Update the invite status
        db_update_invite_status(invite_id, new_status.clone()).await?;

        // If accepted, add user to the server
        if accept {
            add_user_to_server(invite.server.id, user_id).await?;
        }

        // Notify the original sender about the response
        let response_message = ServerMessage::ServerInviteResponse {
            invite_id,
            accepted: accept,
            user: User {
                id: user_id,
                username: "User".to_string(), // We'll need to get the actual user data
                color: Color::White,
                role: common::UserRole::User,
                profile_pic: None,
                cover_banner: None,
                status: common::UserStatus::Connected,
            },
        };
        
        BroadcastService::send_to_user(peer_map, invite.from_user.id, &response_message).await;
        
        info!("Server invite {} by user {}", 
              if accept { "accepted" } else { "declined" }, 
              user_id);

        // Return updated invite
        let mut updated_invite = invite;
        updated_invite.status = new_status;
        Ok(updated_invite)
    }

    /// Get pending invites for a user
    pub async fn get_pending_invites(user_id: Uuid) -> Result<Vec<ServerInvite>> {
        db_get_pending_invites_for_user(user_id).await
    }

    /// Get an invite by its ID
    pub async fn get_invite_by_id(invite_id: Uuid) -> Result<Option<ServerInvite>> {
        db_get_invite_by_id(invite_id).await
    }
}