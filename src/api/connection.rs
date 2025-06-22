use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use crate::errors::Result;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use tracing::error;
use common::{ClientMessage, ServerMessage};

use crate::db;
use crate::services::{UserService, ChatService, NotificationService, BroadcastService};
use regex;

/// Represents a connected peer/client
pub struct Peer {
    pub user_id: Option<Uuid>,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

/// Thread-safe map of all connected peers
pub type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;

/// Main connection handler - processes client connections and messages
pub async fn handle_connection(
    stream: TcpStream,
    peer_map: PeerMap,
) -> Result<()> {
    let peer_id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::unbounded_channel();

    {
        let mut peers = peer_map.lock().await;
        peers.insert(
            peer_id,
            Peer {
                user_id: None,
                tx: tx.clone(),
            },
        );
    }

    let framed = Framed::new(stream, LengthDelimitedCodec::new());
    let (mut sink, mut stream) = framed.split();

    let peer_map_task = peer_map.clone();
    tokio::spawn(async move {
        let mut current_user: Option<common::User> = None;
        
        loop {
            tokio::select! {
                Some(Ok(msg)) = stream.next() => {
                    match bincode::deserialize::<ClientMessage>(&msg) {
                        Ok(message) => {
                            tracing::info!("Parsed ClientMessage: {:?}", message);
                            
                            match message {
                                ClientMessage::Register { username, password } => {
                                    match UserService::register(&username, &password, &peer_map_task).await {
                                        Ok(user) => {
                                            // Update peer map
                                            let mut peers = peer_map_task.lock().await;
                                            if let Some(peer) = peers.get_mut(&peer_id) {
                                                peer.user_id = Some(user.id);
                                            }
                                            current_user = Some(user.clone());
                                            let response = ServerMessage::AuthSuccess(user);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::AuthFailure(e.to_string());
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::Login { username, password } => {
                                    match UserService::login(&username, &password, &peer_map_task).await {
                                        Ok(user) => {
                                            // Update peer map
                                            let mut peers = peer_map_task.lock().await;
                                            if let Some(peer) = peers.get_mut(&peer_id) {
                                                peer.user_id = Some(user.id);
                                            }
                                            current_user = Some(user.clone());
                                            let response = ServerMessage::AuthSuccess(user);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::AuthFailure(e.to_string());
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::Logout => {
                                    if let Some(user) = &current_user {
                                        UserService::logout(user, &peer_map_task).await;
                                    }
                                    
                                    let mut peers = peer_map_task.lock().await;
                                    if let Some(peer) = peers.get_mut(&peer_id) {
                                        peer.user_id = None;
                                    }
                                    current_user = None;
                                }
                                ClientMessage::UpdatePassword(new_password) => {
                                    if let Some(user) = &current_user {
                                        let _ = UserService::update_password(user.id, &new_password).await;
                                    }
                                }
                                ClientMessage::UpdateColor(color) => {
                                    if let Some(user) = &current_user {
                                        // UserColor already contains a String, so we can use it directly
                                        let color_str = color.0;
                                        if let Ok(updated_user) = UserService::update_color(user.id, &color_str, &peer_map_task).await {
                                            current_user = Some(updated_user);
                                        }
                                    }
                                }
                                ClientMessage::UpdateProfile { bio, url1, url2, url3, location, profile_pic, cover_banner } => {
                                    if let Some(user) = &current_user {
                                        let _ = UserService::update_profile(
                                            user.id, bio, url1, url2, url3, location, profile_pic, cover_banner, &peer_map_task
                                        ).await;
                                    }
                                }
                                ClientMessage::GetProfile { user_id } => {
                                    match UserService::get_profile(user_id).await {
                                        Ok(profile) => {
                                            let response = ServerMessage::Profile(profile);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::Notification(format!("Failed to load profile: {}", e), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::GetUserList => {
                                    match UserService::get_user_list(&peer_map_task).await {
                                        Ok(users) => {
                                            let response = ServerMessage::UserList(users);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(_) => {
                                            let response = ServerMessage::Notification("Failed to get user list".to_string(), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::SendChannelMessage { channel_id, content } => {
                                    if let Some(user) = &current_user {
                                        let _ = ChatService::send_channel_message(channel_id, user, &content, &peer_map_task).await;
                                    }
                                }
                                ClientMessage::SendDirectMessage { to, content } => {
                                    if let Some(user) = &current_user {
                                        let _ = ChatService::send_direct_message(user, to, &content, &peer_map_task).await;
                                    }
                                }
                                ClientMessage::SendServerInvite { to_user_id, server_id } => {
                                    if let Some(user) = &current_user {
                                        match crate::services::InviteService::send_server_invite(user.id, to_user_id, server_id, &peer_map_task).await {
                                            Ok(_) => {
                                                let response = ServerMessage::Notification("Server invite sent successfully!".to_string(), false);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to send invite: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::RespondToServerInvite { invite_id, accept } => {
                                    if let Some(user) = &current_user {
                                        match crate::services::InviteService::respond_to_invite(invite_id, user.id, accept, &peer_map_task).await {
                                            Ok(_) => {
                                                let action = if accept { "accepted" } else { "declined" };
                                                let response = ServerMessage::Notification(format!("Server invite {} successfully!", action), false);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                                
                                                // If accepted, refresh the user's server list
                                                if accept {
                                                    let servers = db::servers::db_get_user_servers(user.id).await.unwrap_or_default();
                                                    let response = ServerMessage::Servers(servers);
                                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                                }
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to respond to invite: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::AcceptServerInviteFromUser { from_user_id } => {
                                    if let Some(user) = &current_user {
                                        match crate::services::InviteService::respond_to_invite_from_user(from_user_id, user.id, true, &peer_map_task).await {
                                            Ok(_) => {
                                                let response = ServerMessage::Notification("Server invite accepted successfully!".to_string(), false);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                                
                                                // Refresh the user's server list
                                                let servers = db::servers::db_get_user_servers(user.id).await.unwrap_or_default();
                                                let response = ServerMessage::Servers(servers);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to accept invite: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::DeclineServerInviteFromUser { from_user_id } => {
                                    if let Some(user) = &current_user {
                                        match crate::services::InviteService::respond_to_invite_from_user(from_user_id, user.id, false, &peer_map_task).await {
                                            Ok(_) => {
                                                let response = ServerMessage::Notification("Server invite declined.".to_string(), false);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to decline invite: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetChannelMessages { channel_id, before } => {
                                    match ChatService::get_channel_messages(channel_id, before, 50).await {
                                        Ok((messages, history_complete)) => {
                                            let response = ServerMessage::ChannelMessages { channel_id, messages, history_complete };
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(_) => {
                                            let response = ServerMessage::Notification("Failed to load messages".to_string(), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::GetDirectMessages { user_id, before } => {
                                    if let Some(user) = &current_user {
                                        match ChatService::get_direct_messages(user.id, user_id, before, 50).await {
                                            Ok((messages, history_complete)) => {
                                                let response = ServerMessage::DirectMessages { user_id, messages, history_complete };
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(_) => {
                                                let response = ServerMessage::Notification("Failed to load DMs".to_string(), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetChannelUserList { channel_id } => {
                                    match ChatService::get_channel_users(channel_id, &peer_map_task).await {
                                        Ok(users) => {
                                            let response = ServerMessage::ChannelUserList { channel_id, users };
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                        Err(_) => {
                                            let response = ServerMessage::Notification("Failed to get channel users".to_string(), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::GetDMUserList => {
                                    if let Some(user) = &current_user {
                                        match ChatService::get_dm_user_list(user.id, &peer_map_task).await {
                                            Ok(users) => {
                                                let response = ServerMessage::DMUserList(users);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(_) => {
                                                let response = ServerMessage::Notification("Failed to get DM user list".to_string(), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetNotifications { before } => {
                                    if let Some(user) = &current_user {
                                        match NotificationService::get_notifications(user.id, before).await {
                                            Ok((notifications, history_complete)) => {
                                                let response = ServerMessage::Notifications { notifications, history_complete };
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(_) => {
                                                let response = ServerMessage::Notification("Failed to get notifications".to_string(), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::MarkNotificationRead { notification_id } => {
                                    let _ = NotificationService::mark_notification_read(notification_id).await;
                                }
                                ClientMessage::GetForums => {
                                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                    let response = ServerMessage::Forums(forums);
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetServers => {
                                    if let Some(user) = &current_user {
                                        let servers = db::servers::db_get_user_servers(user.id).await.unwrap_or_default();
                                        let response = ServerMessage::Servers(servers);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::CreatePost { thread_id, content } => {
                                    if let Some(user) = &current_user {
                                        match db::forums::db_create_post(thread_id, user.id, &content).await {
                                            Ok(_) => {
                                                let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                                let response = ServerMessage::Forums(forums);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to create post: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::CreateThread { forum_id, title, content } => {
                                    if let Some(user) = &current_user {
                                        match db::forums::db_create_thread(forum_id, &title, user.id, &content).await {
                                            Ok(_) => {
                                                let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                                let response = ServerMessage::Forums(forums);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to create thread: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::DeletePost(post_id) => {
                                    if let Some(user) = &current_user {
                                        match db::forums::db_delete_post(post_id, user.id).await {
                                            Ok(_) => {
                                                let response = ServerMessage::Notification("Post deleted successfully".to_string(), false);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                                
                                                // Refresh forums to show updated state
                                                let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                                let response = ServerMessage::Forums(forums);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to delete post: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    } else {
                                        let response = ServerMessage::Notification("Must be logged in to delete posts".to_string(), true);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::DeleteThread(thread_id) => {
                                    if let Some(user) = &current_user {
                                        match db::forums::db_delete_thread(thread_id, user.id).await {
                                            Ok(_) => {
                                                let response = ServerMessage::Notification("Thread deleted successfully".to_string(), false);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                                
                                                // Refresh forums to show updated state
                                                let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                                let response = ServerMessage::Forums(forums);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to delete thread: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    } else {
                                        let response = ServerMessage::Notification("Must be logged in to delete threads".to_string(), true);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                // --- ENHANCED PAGINATION HANDLERS ---
                                ClientMessage::GetChannelMessagesPaginated { channel_id, cursor, limit, direction } => {
                                    use crate::services::chat_service::{PaginationRequest, PaginationCursor as ServiceCursor, PaginationDirection as ServiceDirection};
                                    
                                    // Convert protocol types to service types
                                    let service_cursor = match cursor {
                                        common::PaginationCursor::Timestamp(ts) => ServiceCursor::Timestamp(ts),
                                        common::PaginationCursor::Offset(offset) => ServiceCursor::Offset(offset),
                                        common::PaginationCursor::Start => ServiceCursor::Start,
                                    };
                                    
                                    let service_direction = match direction {
                                        common::PaginationDirection::Forward => ServiceDirection::Forward,
                                        common::PaginationDirection::Backward => ServiceDirection::Backward,
                                    };
                                    
                                    let request = PaginationRequest {
                                        cursor: service_cursor,
                                        limit: limit.unwrap_or(50),
                                        direction: service_direction,
                                    };
                                    
                                    let start_time = std::time::Instant::now();
                                    match ChatService::get_channel_messages_paginated(channel_id, request, None).await {
                                        Ok(response) => {
                                            let query_time = start_time.elapsed().as_millis() as u64;
                                            let message_count = response.items.len();
                                            
                                            // Convert back to protocol types
                                            let next_cursor = response.next_cursor.map(|c| match c {
                                                ServiceCursor::Timestamp(ts) => common::PaginationCursor::Timestamp(ts),
                                                ServiceCursor::Offset(offset) => common::PaginationCursor::Offset(offset),
                                                ServiceCursor::Start => common::PaginationCursor::Start,
                                            });
                                            
                                            let prev_cursor = response.prev_cursor.map(|c| match c {
                                                ServiceCursor::Timestamp(ts) => common::PaginationCursor::Timestamp(ts),
                                                ServiceCursor::Offset(offset) => common::PaginationCursor::Offset(offset),
                                                ServiceCursor::Start => common::PaginationCursor::Start,
                                            });
                                            
                                            let response_msg = ServerMessage::ChannelMessagesPaginated {
                                                channel_id,
                                                messages: response.items,
                                                has_more: response.has_more,
                                                next_cursor,
                                                prev_cursor,
                                                total_count: response.total_count,
                                            };
                                            let _ = sink.send(bincode::serialize(&response_msg).unwrap().into()).await;
                                            
                                            // Send performance metrics
                                            let perf_msg = ServerMessage::PerformanceMetrics {
                                                query_time_ms: query_time,
                                                cache_hit_rate: 0.0, // Would need cache stats
                                                message_count,
                                            };
                                            let _ = sink.send(bincode::serialize(&perf_msg).unwrap().into()).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::Notification(format!("Failed to load messages: {}", e), true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::GetDirectMessagesPaginated { user_id, cursor, limit, direction } => {
                                    if let Some(user) = &current_user {
                                        use crate::services::chat_service::{PaginationRequest, PaginationCursor as ServiceCursor, PaginationDirection as ServiceDirection};
                                        
                                        let service_cursor = match cursor {
                                            common::PaginationCursor::Timestamp(ts) => ServiceCursor::Timestamp(ts),
                                            common::PaginationCursor::Offset(offset) => ServiceCursor::Offset(offset),
                                            common::PaginationCursor::Start => ServiceCursor::Start,
                                        };
                                        
                                        let service_direction = match direction {
                                            common::PaginationDirection::Forward => ServiceDirection::Forward,
                                            common::PaginationDirection::Backward => ServiceDirection::Backward,
                                        };
                                        
                                        let request = PaginationRequest {
                                            cursor: service_cursor,
                                            limit: limit.unwrap_or(50),
                                            direction: service_direction,
                                        };
                                        
                                        let start_time = std::time::Instant::now();
                                        match ChatService::get_direct_messages_paginated(user.id, user_id, request, None).await {
                                            Ok(response) => {
                                                let query_time = start_time.elapsed().as_millis() as u64;
                                                let message_count = response.items.len();
                                                
                                                let next_cursor = response.next_cursor.map(|c| match c {
                                                    ServiceCursor::Timestamp(ts) => common::PaginationCursor::Timestamp(ts),
                                                    ServiceCursor::Offset(offset) => common::PaginationCursor::Offset(offset),
                                                    ServiceCursor::Start => common::PaginationCursor::Start,
                                                });
                                                
                                                let prev_cursor = response.prev_cursor.map(|c| match c {
                                                    ServiceCursor::Timestamp(ts) => common::PaginationCursor::Timestamp(ts),
                                                    ServiceCursor::Offset(offset) => common::PaginationCursor::Offset(offset),
                                                    ServiceCursor::Start => common::PaginationCursor::Start,
                                                });
                                                
                                                let response_msg = ServerMessage::DirectMessagesPaginated {
                                                    user_id,
                                                    messages: response.items,
                                                    has_more: response.has_more,
                                                    next_cursor,
                                                    prev_cursor,
                                                    total_count: response.total_count,
                                                };
                                                let _ = sink.send(bincode::serialize(&response_msg).unwrap().into()).await;
                                                
                                                let perf_msg = ServerMessage::PerformanceMetrics {
                                                    query_time_ms: query_time,
                                                    cache_hit_rate: 0.0,
                                                    message_count,
                                                };
                                                let _ = sink.send(bincode::serialize(&perf_msg).unwrap().into()).await;
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to load DMs: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetCacheStats => {
                                    // This would typically be handled by a cache service
                                    // For now, return mock data
                                    let response = ServerMessage::CacheStats {
                                        total_entries: 0,
                                        total_size_mb: 0.0,
                                        hit_ratio: 0.0,
                                        expired_entries: 0,
                                    };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::InvalidateImageCache { keys } => {
                                    // Broadcast cache invalidation to all connected clients
                                    let response = ServerMessage::ImageCacheInvalidated { keys };
                                    BroadcastService::broadcast_to_all(&peer_map_task, &response).await;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Error parsing message: {:?}", e);
                        }
                    }
                }
                Some(msg) = rx.recv() => {
                    tracing::info!("Sending ServerMessage: {:?}", msg);
                    if let Err(e) = sink.send(bincode::serialize(&msg).unwrap().into()).await {
                        error!("Error sending message: {:?}", e);
                        break;
                    }
                }
                else => { break; }
            }
        }
        
        // Cleanup on disconnect
        let mut peers = peer_map_task.lock().await;
        let user_id_opt = peers.get(&peer_id).and_then(|p| p.user_id);
        peers.remove(&peer_id);
        drop(peers);
        
        if let Some(user_id) = user_id_opt {
            if let Ok(profile) = db::users::db_get_user_by_id(user_id).await {
                let user = common::User {
                    id: profile.id,
                    username: profile.username,
                    color: profile.color,
                    role: profile.role,
                    profile_pic: profile.profile_pic,
                    cover_banner: profile.cover_banner,
                    status: common::UserStatus::Connected,
                };
                BroadcastService::broadcast_user_status_change(&peer_map_task, &user, false).await;
            }
        }
    });
    
    Ok(())
}
