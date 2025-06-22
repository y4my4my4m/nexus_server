use super::MessageRouter;
use crate::db::{channels, messages};
use common::{ServerMessage, User, PaginationCursor, PaginationDirection};
use tokio::sync::mpsc;
use uuid::Uuid;

impl MessageRouter {
    /// Handle sending channel message
    pub async fn handle_send_channel_message(
        &self,
        current_user: &Option<User>,
        channel_id: Uuid,
        content: String,
        _response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            let _ = crate::services::ChatService::send_channel_message(channel_id, user, &content, &self.peer_map).await;
        }
        Ok(())
    }

    /// Handle sending direct message
    pub async fn handle_send_direct_message(
        &self,
        current_user: &Option<User>,
        to: Uuid,
        content: String,
        _response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            let _ = crate::services::ChatService::send_direct_message(user, to, &content, &self.peer_map).await;
        }
        Ok(())
    }

    /// Handle get channel messages (legacy)
    pub async fn handle_get_channel_messages(
        &self,
        channel_id: Uuid,
        before: Option<i64>,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        match crate::services::ChatService::get_channel_messages(channel_id, before, 50).await {
            Ok((messages, history_complete)) => {
                let _ = response_sender.send(ServerMessage::ChannelMessages { 
                    channel_id, 
                    messages, 
                    history_complete 
                });
            }
            Err(_) => {
                let _ = response_sender.send(ServerMessage::Notification(
                    "Failed to load messages".to_string(), 
                    true
                ));
            }
        }
        Ok(())
    }

    /// Handle get direct messages (legacy)
    pub async fn handle_get_direct_messages(
        &self,
        current_user: &Option<User>,
        user_id: Uuid,
        before: Option<i64>,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match crate::services::ChatService::get_direct_messages(user.id, user_id, before, 50).await {
                Ok((messages, history_complete)) => {
                    let _ = response_sender.send(ServerMessage::DirectMessages { 
                        user_id, 
                        messages, 
                        history_complete 
                    });
                }
                Err(_) => {
                    let _ = response_sender.send(ServerMessage::Notification(
                        "Failed to load DMs".to_string(), 
                        true
                    ));
                }
            }
        }
        Ok(())
    }

    /// Handle get channel user list - optimized version
    pub async fn handle_get_channel_user_list(
        &self,
        channel_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        // Always use lightweight version for better performance
        match channels::db_get_channel_user_list_lightweight(channel_id).await {
            Ok(user_infos) => {
                // Convert UserInfo to User without profile images for better performance
                let users = user_infos.into_iter().map(|info| User {
                    id: info.id,
                    username: info.username,
                    color: info.color,
                    role: info.role,
                    profile_pic: None, // Exclude for performance
                    cover_banner: None, // Exclude for performance
                    status: info.status,
                }).collect();
                
                let _ = response_sender.send(ServerMessage::ChannelUserList { channel_id, users });
            }
            Err(e) => {
                let error_msg = format!("Failed to get channel users: {}", e);
                let _ = response_sender.send(ServerMessage::Notification(error_msg, true));
            }
        }
        Ok(())
    }

    /// Handle DM user list request - optimized version
    pub async fn handle_get_dm_user_list(
        &self,
        user_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        // Always use lightweight version for better performance
        match messages::db_get_dm_user_list_lightweight(user_id).await {
            Ok(user_infos) => {
                // Convert UserInfo to User without profile images for better performance
                let users = user_infos.into_iter().map(|info| User {
                    id: info.id,
                    username: info.username,
                    color: info.color,
                    role: info.role,
                    profile_pic: None, // Exclude for performance
                    cover_banner: None, // Exclude for performance
                    status: info.status,
                }).collect();
                
                let _ = response_sender.send(ServerMessage::DMUserList(users));
            }
            Err(e) => {
                let error_msg = format!("Failed to get DM users: {}", e);
                let _ = response_sender.send(ServerMessage::Notification(error_msg, true));
            }
        }
        Ok(())
    }

    /// Handle channel messages with enhanced pagination
    pub async fn handle_get_channel_messages_paginated(
        &self,
        channel_id: Uuid,
        cursor: PaginationCursor,
        limit: Option<usize>,
        direction: PaginationDirection,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        let limit = limit.unwrap_or(50).min(200); // Safety limit to prevent abuse
        let reverse_order = matches!(direction, PaginationDirection::Backward);
        
        let before = match cursor {
            PaginationCursor::Timestamp(ts) => Some(ts),
            PaginationCursor::Start => None,
            PaginationCursor::Offset(_) => {
                let _ = response_sender.send(ServerMessage::Notification(
                    "Offset pagination not supported for messages".to_string(), 
                    true
                ));
                return Ok(());
            }
        };

        match channels::db_get_channel_messages_by_timestamp(channel_id, before, limit, reverse_order).await {
            Ok((messages, has_more)) => {
                let next_cursor = if has_more && !messages.is_empty() {
                    match direction {
                        PaginationDirection::Forward => Some(PaginationCursor::Timestamp(messages.last().unwrap().timestamp)),
                        PaginationDirection::Backward => Some(PaginationCursor::Timestamp(messages.first().unwrap().timestamp)),
                    }
                } else {
                    None
                };

                let prev_cursor = if !messages.is_empty() {
                    match direction {
                        PaginationDirection::Forward => Some(PaginationCursor::Timestamp(messages.first().unwrap().timestamp)),
                        PaginationDirection::Backward => Some(PaginationCursor::Timestamp(messages.last().unwrap().timestamp)),
                    }
                } else {
                    None
                };

                // Only get total count for small requests to avoid performance impact
                let total_count = if limit <= 50 {
                    channels::db_get_channel_message_count(channel_id).await.ok()
                } else {
                    None
                };

                let _ = response_sender.send(ServerMessage::ChannelMessagesPaginated {
                    channel_id,
                    messages,
                    has_more,
                    next_cursor,
                    prev_cursor,
                    total_count,
                });
            }
            Err(e) => {
                let error_msg = format!("Failed to get channel messages: {}", e);
                let _ = response_sender.send(ServerMessage::Notification(error_msg, true));
            }
        }
        Ok(())
    }

    /// Handle direct messages with enhanced pagination
    pub async fn handle_get_direct_messages_paginated(
        &self,
        current_user_id: Uuid,
        other_user_id: Uuid,
        cursor: PaginationCursor,
        limit: Option<usize>,
        direction: PaginationDirection,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        let limit = limit.unwrap_or(50).min(200); // Safety limit to prevent abuse
        let reverse_order = matches!(direction, PaginationDirection::Backward);
        
        let before = match cursor {
            PaginationCursor::Timestamp(ts) => Some(ts),
            PaginationCursor::Start => None,
            PaginationCursor::Offset(_) => {
                let _ = response_sender.send(ServerMessage::Notification(
                    "Offset pagination not supported for messages".to_string(), 
                    true
                ));
                return Ok(());
            }
        };

        match messages::db_get_direct_messages_by_timestamp(current_user_id, other_user_id, before, limit, reverse_order).await {
            Ok((messages, has_more)) => {
                let next_cursor = if has_more && !messages.is_empty() {
                    match direction {
                        PaginationDirection::Forward => Some(PaginationCursor::Timestamp(messages.last().unwrap().timestamp)),
                        PaginationDirection::Backward => Some(PaginationCursor::Timestamp(messages.first().unwrap().timestamp)),
                    }
                } else {
                    None
                };

                let prev_cursor = if !messages.is_empty() {
                    match direction {
                        PaginationDirection::Forward => Some(PaginationCursor::Timestamp(messages.first().unwrap().timestamp)),
                        PaginationDirection::Backward => Some(PaginationCursor::Timestamp(messages.last().unwrap().timestamp)),
                    }
                } else {
                    None
                };

                // Only get total count for small requests to avoid performance impact
                let total_count = if limit <= 50 {
                    messages::db_get_direct_message_count(current_user_id, other_user_id).await.ok()
                } else {
                    None
                };

                let _ = response_sender.send(ServerMessage::DirectMessagesPaginated {
                    user_id: other_user_id,
                    messages,
                    has_more,
                    next_cursor,
                    prev_cursor,
                    total_count,
                });
            }
            Err(e) => {
                let error_msg = format!("Failed to get direct messages: {}", e);
                let _ = response_sender.send(ServerMessage::Notification(error_msg, true));
            }
        }
        Ok(())
    }
}