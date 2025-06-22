use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use crate::errors::Result;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use tracing::error;
use common::{ClientMessage, ServerMessage, config::ServerConfig};

use crate::db;
use crate::services::{UserService, ChatService, NotificationService, BroadcastService, RateLimitService, ContentFilterService};
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
    addr: std::net::SocketAddr,
    peer_map: PeerMap,
    rate_limit_service: Arc<RateLimitService>,
    content_filter_service: Arc<RwLock<ContentFilterService>>,
    server_config: ServerConfig,
) -> Result<()> {
    use std::sync::Arc;
    use tokio::sync::RwLock;
    
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
                    // Apply rate limiting
                    if let Err(e) = rate_limit_service.check_request_rate_limit(addr.ip()).await {
                        let response = ServerMessage::Notification(e, true);
                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                        continue;
                    }
                    
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
                                        // The color is now a UserColor, extract the string
                                        let color_str = color.as_str();
                                        if let Ok(updated_user) = UserService::update_color(user.id, color_str, &peer_map_task).await {
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
                                        // Apply rate limiting for messages
                                        if let Err(e) = rate_limit_service.check_message_rate_limit(user.id).await {
                                            let response = ServerMessage::Notification(e, true);
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            continue;
                                        }
                                        
                                        // Apply content filtering
                                        let filter_service = content_filter_service.read().await;
                                        match filter_service.filter_message(&content, user.id) {
                                            crate::services::FilterResult::Allowed => {
                                                drop(filter_service);
                                                let _ = ChatService::send_channel_message(channel_id, user, &content, &peer_map_task).await;
                                            }
                                            crate::services::FilterResult::Blocked { reason } => {
                                                let response = ServerMessage::Notification(reason, true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                            crate::services::FilterResult::Flagged { reason } => {
                                                // Log for manual review but allow the message
                                                tracing::warn!("Message flagged for review from user {}: {}", user.id, reason);
                                                drop(filter_service);
                                                let _ = ChatService::send_channel_message(channel_id, user, &content, &peer_map_task).await;
                                            }
                                        }
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
                                ClientMessage::DeletePost(_) | ClientMessage::DeleteThread(_) => {
                                    // TODO: Implement delete functionality
                                    let response = ServerMessage::Notification("Delete functionality not yet implemented".to_string(), true);
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
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
