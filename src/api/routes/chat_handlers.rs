use super::MessageRouter;
use crate::services::ChatService;
use crate::services::chat_service::{PaginationRequest, PaginationCursor, PaginationDirection};
use common::{ServerMessage, User, PaginationCursor as CommonCursor, PaginationDirection as CommonDirection};
use tokio::sync::mpsc;
use uuid::Uuid;
use std::time::Instant;

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
            let _ = ChatService::send_channel_message(channel_id, user, &content, &self.peer_map).await;
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
            let _ = ChatService::send_direct_message(user, to, &content, &self.peer_map).await;
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
        match ChatService::get_channel_messages(channel_id, before, 50).await {
            Ok((messages, history_complete)) => {
                self.send_response(response_sender, ServerMessage::ChannelMessages { 
                    channel_id, 
                    messages, 
                    history_complete 
                });
            }
            Err(_) => {
                self.send_error(response_sender, "Failed to load messages");
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
            match ChatService::get_direct_messages(user.id, user_id, before, 50).await {
                Ok((messages, history_complete)) => {
                    self.send_response(response_sender, ServerMessage::DirectMessages { 
                        user_id, 
                        messages, 
                        history_complete 
                    });
                }
                Err(_) => {
                    self.send_error(response_sender, "Failed to load DMs");
                }
            }
        }
        Ok(())
    }

    /// Handle get channel messages with pagination
    pub async fn handle_get_channel_messages_paginated(
        &self,
        channel_id: Uuid,
        cursor: CommonCursor,
        limit: Option<usize>,
        direction: CommonDirection,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        let start_time = Instant::now();
        
        // Convert protocol types to service types
        let service_cursor = match cursor {
            CommonCursor::Timestamp(ts) => PaginationCursor::Timestamp(ts),
            CommonCursor::Offset(offset) => PaginationCursor::Offset(offset),
            CommonCursor::Start => PaginationCursor::Start,
        };
        
        let service_direction = match direction {
            CommonDirection::Forward => PaginationDirection::Forward,
            CommonDirection::Backward => PaginationDirection::Backward,
        };
        
        let request = PaginationRequest {
            cursor: service_cursor,
            limit: limit.unwrap_or(50),
            direction: service_direction,
        };
        
        match ChatService::get_channel_messages_paginated(channel_id, request, None).await {
            Ok(response) => {
                let query_time = start_time.elapsed().as_millis() as f64;
                let message_count = response.items.len();
                
                // Convert back to protocol types
                let next_cursor = response.next_cursor.map(|c| match c {
                    PaginationCursor::Timestamp(ts) => CommonCursor::Timestamp(ts),
                    PaginationCursor::Offset(offset) => CommonCursor::Offset(offset),
                    PaginationCursor::Start => CommonCursor::Start,
                });
                
                let prev_cursor = response.prev_cursor.map(|c| match c {
                    PaginationCursor::Timestamp(ts) => CommonCursor::Timestamp(ts),
                    PaginationCursor::Offset(offset) => CommonCursor::Offset(offset),
                    PaginationCursor::Start => CommonCursor::Start,
                });
                
                self.send_response(response_sender, ServerMessage::ChannelMessagesPaginated {
                    channel_id,
                    messages: response.items,
                    has_more: response.has_more,
                    next_cursor,
                    prev_cursor,
                    total_count: response.total_count,
                });
                
                // Send performance metrics
                self.send_response(response_sender, ServerMessage::PerformanceMetrics {
                    query_time_ms: query_time as u64,
                    cache_hit_rate: 0.0,
                    message_count,
                });
            }
            Err(e) => {
                self.send_error(response_sender, &format!("Failed to load messages: {}", e));
            }
        }
        Ok(())
    }

    /// Handle get direct messages with pagination
    pub async fn handle_get_direct_messages_paginated(
        &self,
        current_user: &Option<User>,
        user_id: Uuid,
        cursor: CommonCursor,
        limit: Option<usize>,
        direction: CommonDirection,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            let start_time = Instant::now();
            
            // Convert protocol types to service types
            let service_cursor = match cursor {
                CommonCursor::Timestamp(ts) => PaginationCursor::Timestamp(ts),
                CommonCursor::Offset(offset) => PaginationCursor::Offset(offset),
                CommonCursor::Start => PaginationCursor::Start,
            };
            
            let service_direction = match direction {
                CommonDirection::Forward => PaginationDirection::Forward,
                CommonDirection::Backward => PaginationDirection::Backward,
            };
            
            let request = PaginationRequest {
                cursor: service_cursor,
                limit: limit.unwrap_or(50),
                direction: service_direction,
            };
            
            match ChatService::get_direct_messages_paginated(user.id, user_id, request, None).await {
                Ok(response) => {
                    let query_time = start_time.elapsed().as_millis() as f64;
                    let message_count = response.items.len();
                    
                    // Convert back to protocol types
                    let next_cursor = response.next_cursor.map(|c| match c {
                        PaginationCursor::Timestamp(ts) => CommonCursor::Timestamp(ts),
                        PaginationCursor::Offset(offset) => CommonCursor::Offset(offset),
                        PaginationCursor::Start => CommonCursor::Start,
                    });
                    
                    let prev_cursor = response.prev_cursor.map(|c| match c {
                        PaginationCursor::Timestamp(ts) => CommonCursor::Timestamp(ts),
                        PaginationCursor::Offset(offset) => CommonCursor::Offset(offset),
                        PaginationCursor::Start => CommonCursor::Start,
                    });
                    
                    self.send_response(response_sender, ServerMessage::DirectMessagesPaginated {
                        user_id,
                        messages: response.items,
                        has_more: response.has_more,
                        next_cursor,
                        prev_cursor,
                        total_count: response.total_count,
                    });
                    
                    self.send_response(response_sender, ServerMessage::PerformanceMetrics {
                        query_time_ms: query_time as u64,
                        cache_hit_rate: 0.0,
                        message_count,
                    });
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to load DMs: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Handle get channel user list
    pub async fn handle_get_channel_user_list(
        &self,
        channel_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        match ChatService::get_channel_users(channel_id, &self.peer_map).await {
            Ok(users) => {
                self.send_response(response_sender, ServerMessage::ChannelUserList { 
                    channel_id, 
                    users 
                });
            }
            Err(_) => {
                self.send_error(response_sender, "Failed to get channel users");
            }
        }
        Ok(())
    }

    /// Handle get DM user list
    pub async fn handle_get_dm_user_list(
        &self,
        current_user: &Option<User>,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match ChatService::get_dm_user_list(user.id, &self.peer_map).await {
                Ok(users) => {
                    self.send_response(response_sender, ServerMessage::DMUserList(users));
                }
                Err(_) => {
                    self.send_error(response_sender, "Failed to get DM user list");
                }
            }
        }
        Ok(())
    }

    /// Handle get servers
    pub async fn handle_get_servers(
        &self,
        current_user: &Option<User>,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            let servers = crate::db::servers::db_get_user_servers(user.id).await.unwrap_or_default();
            self.send_response(response_sender, ServerMessage::Servers(servers));
        }
        Ok(())
    }
}