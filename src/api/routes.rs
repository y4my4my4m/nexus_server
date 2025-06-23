use crate::api::connection::PeerMap;
use crate::errors::Result;
use common::{ClientMessage, ServerMessage, User};
use tokio::sync::mpsc;
use tracing::error;
use uuid::Uuid;

/// Message router that dispatches client messages to appropriate handlers
pub struct MessageRouter {
    peer_map: PeerMap,
}

impl MessageRouter {
    pub fn new(peer_map: PeerMap) -> Self {
        Self { peer_map }
    }

    /// Route and handle a client message
    pub async fn handle_message(
        &self,
        message: ClientMessage,
        current_user: &mut Option<User>,
        peer_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> Result<()> {
        match message {
            // Authentication messages
            ClientMessage::Register { username, password } => {
                self.handle_register(username, password, current_user, peer_id, response_sender).await
            }
            ClientMessage::Login { username, password } => {
                self.handle_login(username, password, current_user, peer_id, response_sender).await
            }
            ClientMessage::Logout => {
                self.handle_logout(current_user, peer_id, response_sender).await
            }

            // User profile messages
            ClientMessage::UpdatePassword(new_password) => {
                self.handle_update_password(current_user, new_password, response_sender).await
            }
            ClientMessage::UpdateColor(color) => {
                self.handle_update_color(current_user, color, response_sender).await
            }
            ClientMessage::UpdateProfile { bio, url1, url2, url3, location, profile_pic, cover_banner } => {
                self.handle_update_profile(current_user, bio, url1, url2, url3, location, profile_pic, cover_banner, response_sender).await
            }
            ClientMessage::GetProfile { user_id } => {
                self.handle_get_profile(user_id, response_sender).await
            }
            ClientMessage::GetUserList => {
                self.handle_get_user_list(response_sender).await
            }

            // Chat messages
            ClientMessage::SendChannelMessage { channel_id, content } => {
                self.handle_send_channel_message(current_user, channel_id, content, response_sender).await
            }
            ClientMessage::SendDirectMessage { to, content } => {
                self.handle_send_direct_message(current_user, to, content, response_sender).await
            }
            ClientMessage::GetChannelMessages { channel_id, before } => {
                self.handle_get_channel_messages(channel_id, before, response_sender).await
            }
            ClientMessage::GetDirectMessages { user_id, before } => {
                self.handle_get_direct_messages(current_user, user_id, before, response_sender).await
            }
            ClientMessage::GetChannelUserList { channel_id } => {
                self.handle_get_channel_user_list(channel_id, response_sender).await
            }
            ClientMessage::GetDMUserList => {
                if let Some(user) = current_user {
                    self.handle_get_dm_user_list(user.id, response_sender).await
                } else {
                    self.send_error(response_sender, "Must be logged in to get DM user list");
                    Ok(())
                }
            }

            // Enhanced pagination messages
            ClientMessage::GetChannelMessagesPaginated { channel_id, cursor, limit, direction } => {
                self.handle_get_channel_messages_paginated(channel_id, cursor, limit, direction, response_sender).await
            }
            ClientMessage::GetDirectMessagesPaginated { user_id, cursor, limit, direction } => {
                if let Some(user) = current_user {
                    self.handle_get_direct_messages_paginated(user.id, user_id, cursor, limit, direction, response_sender).await
                } else {
                    self.send_error(response_sender, "Must be logged in to get direct messages");
                    Ok(())
                }
            }

            // Server and forum messages
            ClientMessage::GetServers => {
                self.handle_get_servers(current_user, response_sender).await
            }
            ClientMessage::GetForums => {
                self.handle_get_forums(response_sender).await
            }
            ClientMessage::CreateForum { name, description } => {
                self.handle_create_forum(current_user, name, description, response_sender).await
            }
            ClientMessage::DeleteForum { forum_id } => {
                self.handle_delete_forum(current_user, forum_id, response_sender).await
            }
            ClientMessage::CreateThread { forum_id, title, content } => {
                self.handle_create_thread(current_user, forum_id, title, content, response_sender).await
            }
            ClientMessage::CreatePost { thread_id, content } => {
                self.handle_create_post(current_user, thread_id, content, response_sender).await
            }
            ClientMessage::CreatePostReply { thread_id, content, reply_to } => {
                self.handle_create_post_reply(current_user, thread_id, content, reply_to, response_sender).await
            }
            ClientMessage::DeletePost(post_id) => {
                self.handle_delete_post(current_user, post_id, response_sender).await
            }
            ClientMessage::DeleteThread(thread_id) => {
                self.handle_delete_thread(current_user, thread_id, response_sender).await
            }

            // Invite messages
            ClientMessage::SendServerInvite { to_user_id, server_id } => {
                self.handle_send_server_invite(current_user, to_user_id, server_id, response_sender).await
            }
            ClientMessage::RespondToServerInvite { invite_id, accept } => {
                self.handle_respond_to_server_invite(current_user, invite_id, accept, response_sender).await
            }
            ClientMessage::AcceptServerInviteFromUser { from_user_id } => {
                self.handle_accept_server_invite_from_user(current_user, from_user_id, response_sender).await
            }
            ClientMessage::DeclineServerInviteFromUser { from_user_id } => {
                self.handle_decline_server_invite_from_user(current_user, from_user_id, response_sender).await
            }

            // Notification messages
            ClientMessage::GetNotifications { before } => {
                self.handle_get_notifications(current_user, before, response_sender).await
            }
            ClientMessage::MarkNotificationRead { notification_id } => {
                self.handle_mark_notification_read(notification_id, response_sender).await
            }

            // Cache and performance messages
            ClientMessage::GetCacheStats => {
                self.handle_get_cache_stats(response_sender).await
            }
            ClientMessage::InvalidateImageCache { keys } => {
                self.handle_invalidate_image_cache(keys, response_sender).await
            }
            ClientMessage::GetUserAvatars { user_ids } => {
                self.handle_get_user_avatars(user_ids, response_sender).await
            }
        }
    }

    // Helper method to send responses
    fn send_response(&self, sender: &mpsc::UnboundedSender<ServerMessage>, message: ServerMessage) {
        if let Err(e) = sender.send(message) {
            error!("Failed to send response: {:?}", e);
        }
    }

    // Helper method to send error notifications
    fn send_error(&self, sender: &mpsc::UnboundedSender<ServerMessage>, error: &str) {
        self.send_response(sender, ServerMessage::Notification(error.to_string(), true));
    }

    // Helper method to send success notifications
    fn send_success(&self, sender: &mpsc::UnboundedSender<ServerMessage>, message: &str) {
        self.send_response(sender, ServerMessage::Notification(message.to_string(), false));
    }
}

// Import handler modules
mod auth_handlers;
mod chat_handlers;
mod forum_handlers;
mod invite_handlers;
mod notification_handlers;
mod cache_handlers;