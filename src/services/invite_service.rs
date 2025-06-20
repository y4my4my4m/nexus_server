use crate::db::invites::*;
use crate::db::servers::{add_user_to_server, db_is_user_in_server};
use crate::errors::{Result, ServerError};
use common::{ServerInvite, ServerInviteStatus, User};
use uuid::Uuid;

pub struct InviteService;

impl InviteService {
    /// Send a server invite to another user
    pub async fn send_server_invite(
        from_user_id: Uuid,
        to_user_id: Uuid,
        server_id: Uuid,
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
        
        Ok(invite_id)
    }

    /// Respond to a server invite (accept or decline)
    pub async fn respond_to_invite(
        invite_id: Uuid,
        user_id: Uuid,
        accept: bool,
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