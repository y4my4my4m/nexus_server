use crate::db::invites::*;
use crate::db::servers::{add_user_to_server, db_is_user_in_server};
use crate::db::users::db_get_user_by_id;
use crate::db::messages;
use crate::errors::{Result, ServerError};
use crate::services::BroadcastService;
use crate::api::connection::PeerMap;
use common::{ServerInvite, ServerInviteStatus, ServerMessage, User, DirectMessage};
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
            // Get sender user info for the DM
            match db_get_user_by_id(from_user_id).await {
                Ok(from_user_profile) => {
                    // Convert UserProfile to User
                    let from_user = User {
                        id: from_user_profile.id,
                        username: from_user_profile.username.clone(),
                        color: from_user_profile.color,
                        role: from_user_profile.role,
                        profile_pic: from_user_profile.profile_pic.clone(),
                        cover_banner: from_user_profile.cover_banner.clone(),
                        status: common::UserStatus::Connected,
                    };
                    
                    let timestamp = chrono::Utc::now().timestamp();
                    
                    // Create special DM content for server invite
                    let invite_content = format!("🎮 SERVER INVITE: {} invited you to join '{}'!\n\nType /accept to accept or /decline to decline this invitation.", 
                        from_user.username, 
                        invite.server.name
                    );
                    
                    // Store the invite message as a DM in the database
                    let dm_id = messages::db_store_direct_message(
                        from_user_id, to_user_id, &invite_content, timestamp
                    ).await.map_err(|e| ServerError::Database(e))?;

                    // Create DM object
                    let dm = DirectMessage {
                        id: dm_id,
                        from: from_user_id,
                        to: to_user_id,
                        timestamp,
                        content: invite_content,
                        author_username: from_user.username.clone(),
                        author_color: from_user.color,
                        author_profile_pic: from_user.profile_pic.clone(),
                    };

                    // Send the DM to both users
                    let dm_message = ServerMessage::DirectMessage(dm);
                    let user_ids = vec![from_user_id, to_user_id];
                    BroadcastService::broadcast_to_users(peer_map, &user_ids, &dm_message).await;

                    // Also send the raw invite data for the client to handle specially
                    let invite_message = ServerMessage::ServerInviteReceived(invite.clone());
                    BroadcastService::send_to_user(peer_map, to_user_id, &invite_message).await;
                    
                    info!("Server invite sent as DM to user {}", to_user_id);
                }
                Err(e) => {
                    error!("Failed to get user profile for invite DM: {:?}", e);
                    return Err(ServerError::Database(e));
                }
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

        // Fetch the actual user data
        let user = db_get_user_by_id(user_id).await?
            .ok_or_else(|| ServerError::NotFound("User not found".to_string()))?;

        // Notify the original sender about the response
        let response_message = ServerMessage::ServerInviteResponse {
            invite_id,
            accepted: accept,
            user: User {
                id: user.id,
                username: user.username,
                color: user.color,
                role: user.role,
                profile_pic: user.profile_pic,
                cover_banner: user.cover_banner,
                status: user.status,
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

    /// Respond to a server invite from a specific user (for /accept and /decline commands in DM)
    pub async fn respond_to_invite_from_user(
        from_user_id: Uuid,
        to_user_id: Uuid,
        accept: bool,
        peer_map: &PeerMap,
    ) -> Result<()> {
        // Find the pending invite from this user
        let invite = db_get_pending_invite_from_user(from_user_id, to_user_id).await?
            .ok_or_else(|| ServerError::NotFound("No pending invite from this user".to_string()))?;

        // Use the existing respond_to_invite method
        Self::respond_to_invite(invite.id, to_user_id, accept, peer_map).await?;
        
        Ok(())
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