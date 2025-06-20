// server/src/api/connection.rs

use crate::services::{UserService, ChatService, NotificationService, BroadcastService};
use crate::errors::{Result, ServerError};
use common::{ClientMessage, ServerMessage, User};
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};
use uuid::Uuid;

/// Represents a connected peer/client
pub struct Peer {
    pub user_id: Option<Uuid>,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

/// Thread-safe map of all connected peers
pub type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;

/// Main connection handler - processes client connections and messages
pub async fn handle_connection(stream: TcpStream, peer_map: PeerMap) -> Result<()> {
    let peer_id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::unbounded_channel();

    // Register peer in the map
    {
        let mut peers = peer_map.lock().await;
        peers.insert(peer_id, Peer {
            user_id: None,
            tx: tx.clone(),
        });
    }

    let framed = Framed::new(stream, LengthDelimitedCodec::new());
    let (mut sink, mut stream) = framed.split();

    let peer_map_clone = peer_map.clone();
    
    // Spawn message sending task
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match bincode::serialize(&msg) {
                Ok(data) => {
                    if sink.send(data.into()).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("Failed to serialize message: {}", e);
                }
            }
        }
    });

    // Main message processing loop
    let mut current_user: Option<User> = None;
    
    while let Some(result) = stream.next().await {
        match result {
            Ok(data) => {
                match bincode::deserialize::<ClientMessage>(&data) {
                    Ok(message) => {
                        info!("Processing message from peer {}: {:?}", peer_id, message);
                        
                        if let Err(e) = process_client_message(
                            message,
                            &mut current_user,
                            peer_id,
                            &peer_map_clone,
                        ).await {
                            error!("Error processing message: {}", e);
                            let error_msg = ServerMessage::Notification(
                                format!("Server error: {}", e),
                                true
                            );
                            if let Err(_) = tx.send(error_msg) {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to deserialize message: {}", e);
                    }
                }
            }
            Err(e) => {
                error!("Connection error: {}", e);
                break;
            }
        }
    }

    // Cleanup on disconnect
    cleanup_connection(peer_id, &current_user, &peer_map_clone).await;
    send_task.abort();
    
    Ok(())
}

/// Process individual client messages using the service layer
async fn process_client_message(
    message: ClientMessage,
    current_user: &mut Option<User>,
    peer_id: Uuid,
    peer_map: &PeerMap,
) -> Result<()> {
    match message {
        // Authentication messages
        ClientMessage::Register { username, password } => {
            handle_register(username, password, current_user, peer_id, peer_map).await
        }
        
        ClientMessage::Login { username, password } => {
            handle_login(username, password, current_user, peer_id, peer_map).await
        }
        
        ClientMessage::Logout => {
            handle_logout(current_user, peer_id, peer_map).await
        }

        // User management messages
        ClientMessage::UpdatePassword(new_password) => {
            handle_update_password(current_user, &new_password, peer_map).await
        }

        ClientMessage::UpdateColor(color) => {
            handle_update_color(current_user, color, peer_id, peer_map).await
        }

        ClientMessage::UpdateProfile { bio, url1, url2, url3, location, profile_pic, cover_banner } => {
            handle_update_profile(
                current_user, bio, url1, url2, url3, location, 
                profile_pic, cover_banner, peer_map
            ).await
        }

        ClientMessage::GetProfile { user_id } => {
            handle_get_profile(user_id, peer_map, peer_id).await
        }

        ClientMessage::GetUserList => {
            handle_get_user_list(peer_map, peer_id).await
        }

        // Chat messages
        ClientMessage::SendChannelMessage { channel_id, content } => {
            handle_send_channel_message(current_user, channel_id, &content, peer_map).await
        }

        ClientMessage::SendDirectMessage { to, content } => {
            handle_send_direct_message(current_user, to, &content, peer_map).await
        }

        ClientMessage::GetChannelMessages { channel_id, before } => {
            handle_get_channel_messages(channel_id, before, peer_map, peer_id).await
        }

        ClientMessage::GetDirectMessages { user_id, before } => {
            handle_get_direct_messages(current_user, user_id, before, peer_map, peer_id).await
        }

        ClientMessage::GetChannelUserList { channel_id } => {
            handle_get_channel_users(channel_id, peer_map, peer_id).await
        }

        ClientMessage::GetDMUserList => {
            handle_get_dm_user_list(current_user, peer_map, peer_id).await
        }

        // Server/Channel management
        ClientMessage::GetServers => {
            handle_get_servers(current_user, peer_map, peer_id).await
        }

        // Notifications
        ClientMessage::GetNotifications { before } => {
            handle_get_notifications(current_user, before, peer_map, peer_id).await
        }

        ClientMessage::MarkNotificationRead { notification_id } => {
            handle_mark_notification_read(notification_id).await
        }

        // Legacy forum support
        ClientMessage::GetForums => {
            handle_get_forums(peer_map, peer_id).await
        }

        ClientMessage::CreatePost { thread_id, content } => {
            handle_create_post(current_user, thread_id, &content, peer_map, peer_id).await
        }

        ClientMessage::CreateThread { forum_id, title, content } => {
            handle_create_thread(current_user, forum_id, &title, &content, peer_map, peer_id).await
        }
    }
}

// Authentication handlers
async fn handle_register(
    username: String,
    password: String,
    current_user: &mut Option<User>,
    peer_id: Uuid,
    peer_map: &PeerMap,
) -> Result<()> {
    match UserService::register(&username, &password, peer_map).await {
        Ok(user) => {
            // Update peer with user ID
            {
                let mut peers = peer_map.lock().await;
                if let Some(peer) = peers.get_mut(&peer_id) {
                    peer.user_id = Some(user.id);
                }
            }
            
            *current_user = Some(user.clone());
            send_to_peer(peer_map, peer_id, ServerMessage::AuthSuccess(user)).await;
        }
        Err(e) => {
            send_to_peer(peer_map, peer_id, ServerMessage::AuthFailure(e.to_string())).await;
        }
    }
    Ok(())
}

async fn handle_login(
    username: String,
    password: String,
    current_user: &mut Option<User>,
    peer_id: Uuid,
    peer_map: &PeerMap,
) -> Result<()> {
    match UserService::login(&username, &password, peer_map).await {
        Ok(user) => {
            // Update peer with user ID
            {
                let mut peers = peer_map.lock().await;
                if let Some(peer) = peers.get_mut(&peer_id) {
                    peer.user_id = Some(user.id);
                }
            }
            
            *current_user = Some(user.clone());
            send_to_peer(peer_map, peer_id, ServerMessage::AuthSuccess(user)).await;
        }
        Err(e) => {
            send_to_peer(peer_map, peer_id, ServerMessage::AuthFailure(e.to_string())).await;
        }
    }
    Ok(())
}

async fn handle_logout(
    current_user: &mut Option<User>,
    peer_id: Uuid,
    peer_map: &PeerMap,
) -> Result<()> {
    if let Some(user) = current_user.take() {
        // Update peer to remove user ID
        {
            let mut peers = peer_map.lock().await;
            if let Some(peer) = peers.get_mut(&peer_id) {
                peer.user_id = None;
            }
        }
        
        UserService::logout(&user, peer_map).await;
    }
    Ok(())
}

// User management handlers
async fn handle_update_password(
    current_user: &Option<User>,
    new_password: &str,
    peer_map: &PeerMap,
) -> Result<()> {
    if let Some(user) = current_user {
        match UserService::update_password(user.id, new_password).await {
            Ok(_) => {
                send_notification(peer_map, user.id, "Password updated successfully", false).await;
            }
            Err(e) => {
                send_notification(peer_map, user.id, &format!("Failed to update password: {}", e), true).await;
            }
        }
    }
    Ok(())
}

async fn handle_update_color(
    current_user: &mut Option<User>,
    color: common::SerializableColor,
    peer_id: Uuid,
    peer_map: &PeerMap,
) -> Result<()> {
    if let Some(user) = current_user {
        let color_str = match color.0 {
            ratatui::style::Color::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
            other => format!("{:?}", other),
        };
        
        match UserService::update_color(user.id, &color_str, peer_map).await {
            Ok(updated_user) => {
                *current_user = Some(updated_user);
                send_notification(peer_map, user.id, "Color updated successfully", false).await;
            }
            Err(e) => {
                send_notification(peer_map, user.id, &format!("Failed to update color: {}", e), true).await;
            }
        }
    }
    Ok(())
}

async fn handle_update_profile(
    current_user: &Option<User>,
    bio: Option<String>,
    url1: Option<String>,
    url2: Option<String>,
    url3: Option<String>,
    location: Option<String>,
    profile_pic: Option<String>,
    cover_banner: Option<String>,
    peer_map: &PeerMap,
) -> Result<()> {
    if let Some(user) = current_user {
        match UserService::update_profile(
            user.id, bio, url1, url2, url3, location, profile_pic, cover_banner, peer_map
        ).await {
            Ok(profile) => {
                send_to_peer(peer_map, get_peer_id_for_user(peer_map, user.id).await.unwrap_or(Uuid::nil()), 
                    ServerMessage::Profile(profile)).await;
            }
            Err(e) => {
                send_notification(peer_map, user.id, &format!("Failed to update profile: {}", e), true).await;
            }
        }
    }
    Ok(())
}

async fn handle_get_profile(
    user_id: Uuid,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    match UserService::get_profile(user_id).await {
        Ok(profile) => {
            send_to_peer(peer_map, peer_id, ServerMessage::Profile(profile)).await;
        }
        Err(e) => {
            send_to_peer(peer_map, peer_id, 
                ServerMessage::Notification(format!("Failed to get profile: {}", e), true)).await;
        }
    }
    Ok(())
}

async fn handle_get_user_list(peer_map: &PeerMap, peer_id: Uuid) -> Result<()> {
    match UserService::get_user_list(peer_map).await {
        Ok(users) => {
            send_to_peer(peer_map, peer_id, ServerMessage::UserList(users)).await;
        }
        Err(e) => {
            send_to_peer(peer_map, peer_id,
                ServerMessage::Notification(format!("Failed to get user list: {}", e), true)).await;
        }
    }
    Ok(())
}

// Chat handlers
async fn handle_send_channel_message(
    current_user: &Option<User>,
    channel_id: Uuid,
    content: &str,
    peer_map: &PeerMap,
) -> Result<()> {
    if let Some(user) = current_user {
        ChatService::send_channel_message(channel_id, user, content, peer_map).await?;
    }
    Ok(())
}

async fn handle_send_direct_message(
    current_user: &Option<User>,
    to_user_id: Uuid,
    content: &str,
    peer_map: &PeerMap,
) -> Result<()> {
    if let Some(user) = current_user {
        ChatService::send_direct_message(user, to_user_id, content, peer_map).await?;
    }
    Ok(())
}

async fn handle_get_channel_messages(
    channel_id: Uuid,
    before: Option<Uuid>,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    match ChatService::get_channel_messages(channel_id, before, 50).await {
        Ok((messages, history_complete)) => {
            send_to_peer(peer_map, peer_id, 
                ServerMessage::ChannelMessages { channel_id, messages, history_complete }).await;
        }
        Err(e) => {
            send_to_peer(peer_map, peer_id,
                ServerMessage::Notification(format!("Failed to get channel messages: {}", e), true)).await;
        }
    }
    Ok(())
}

async fn handle_get_direct_messages(
    current_user: &Option<User>,
    other_user_id: Uuid,
    before: Option<i64>,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    if let Some(user) = current_user {
        match ChatService::get_direct_messages(user.id, other_user_id, before, 20).await {
            Ok((messages, history_complete)) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::DirectMessages { 
                        user_id: other_user_id, 
                        messages, 
                        history_complete 
                    }).await;
            }
            Err(e) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::Notification(format!("Failed to get direct messages: {}", e), true)).await;
            }
        }
    }
    Ok(())
}

async fn handle_get_channel_users(
    channel_id: Uuid,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    match ChatService::get_channel_users(channel_id, peer_map).await {
        Ok(users) => {
            send_to_peer(peer_map, peer_id,
                ServerMessage::ChannelUserList { channel_id, users }).await;
        }
        Err(e) => {
            send_to_peer(peer_map, peer_id,
                ServerMessage::Notification(format!("Failed to get channel users: {}", e), true)).await;
        }
    }
    Ok(())
}

async fn handle_get_dm_user_list(
    current_user: &Option<User>,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    if let Some(user) = current_user {
        match ChatService::get_dm_user_list(user.id, peer_map).await {
            Ok(users) => {
                send_to_peer(peer_map, peer_id, ServerMessage::DMUserList(users)).await;
            }
            Err(e) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::Notification(format!("Failed to get DM user list: {}", e), true)).await;
            }
        }
    }
    Ok(())
}

// Server management handlers
async fn handle_get_servers(
    current_user: &Option<User>,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    if let Some(user) = current_user {
        match crate::db::servers::db_get_user_servers(user.id).await {
            Ok(servers) => {
                send_to_peer(peer_map, peer_id, ServerMessage::Servers(servers)).await;
            }
            Err(e) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::Notification(format!("Failed to get servers: {}", e), true)).await;
            }
        }
    }
    Ok(())
}

// Notification handlers
async fn handle_get_notifications(
    current_user: &Option<User>,
    before: Option<i64>,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    if let Some(user) = current_user {
        match NotificationService::get_notifications(user.id, before).await {
            Ok((notifications, history_complete)) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::Notifications { notifications, history_complete }).await;
            }
            Err(e) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::Notification(format!("Failed to get notifications: {}", e), true)).await;
            }
        }
    }
    Ok(())
}

async fn handle_mark_notification_read(notification_id: Uuid) -> Result<()> {
    NotificationService::mark_notification_read(notification_id).await?;
    Ok(())
}

// Legacy forum handlers (for backward compatibility)
async fn handle_get_forums(peer_map: &PeerMap, peer_id: Uuid) -> Result<()> {
    match crate::db::forums::db_get_forums().await {
        Ok(forums) => {
            send_to_peer(peer_map, peer_id, ServerMessage::Forums(forums)).await;
        }
        Err(e) => {
            send_to_peer(peer_map, peer_id,
                ServerMessage::Notification(format!("Failed to get forums: {}", e), true)).await;
        }
    }
    Ok(())
}

async fn handle_create_post(
    current_user: &Option<User>,
    thread_id: Uuid,
    content: &str,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    if let Some(user) = current_user {
        match crate::db::forums::db_create_post(thread_id, user.id, content).await {
            Ok(_) => {
                // Refresh forums for the user
                if let Ok(forums) = crate::db::forums::db_get_forums().await {
                    send_to_peer(peer_map, peer_id, ServerMessage::Forums(forums)).await;
                }
            }
            Err(e) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::Notification(format!("Failed to create post: {}", e), true)).await;
            }
        }
    }
    Ok(())
}

async fn handle_create_thread(
    current_user: &Option<User>,
    forum_id: Uuid,
    title: &str,
    content: &str,
    peer_map: &PeerMap,
    peer_id: Uuid,
) -> Result<()> {
    if let Some(user) = current_user {
        match crate::db::forums::db_create_thread(forum_id, title, user.id, content).await {
            Ok(_) => {
                // Refresh forums for all users
                if let Ok(forums) = crate::db::forums::db_get_forums().await {
                    BroadcastService::broadcast_to_all(peer_map, &ServerMessage::Forums(forums)).await;
                }
            }
            Err(e) => {
                send_to_peer(peer_map, peer_id,
                    ServerMessage::Notification(format!("Failed to create thread: {}", e), true)).await;
            }
        }
    }
    Ok(())
}

// Helper functions
async fn send_to_peer(peer_map: &PeerMap, peer_id: Uuid, message: ServerMessage) {
    let peers = peer_map.lock().await;
    if let Some(peer) = peers.get(&peer_id) {
        let _ = peer.tx.send(message);
    }
}

async fn send_notification(peer_map: &PeerMap, user_id: Uuid, message: &str, is_error: bool) {
    let notification = ServerMessage::Notification(message.to_string(), is_error);
    BroadcastService::send_to_user(peer_map, user_id, &notification).await;
}

async fn get_peer_id_for_user(peer_map: &PeerMap, user_id: Uuid) -> Option<Uuid> {
    let peers = peer_map.lock().await;
    for (peer_id, peer) in peers.iter() {
        if peer.user_id == Some(user_id) {
            return Some(*peer_id);
        }
    }
    None
}

async fn cleanup_connection(
    peer_id: Uuid,
    current_user: &Option<User>,
    peer_map: &PeerMap,
) {
    // Remove peer from map
    {
        let mut peers = peer_map.lock().await;
        peers.remove(&peer_id);
    }

    // Handle user logout if they were authenticated
    if let Some(user) = current_user {
        UserService::logout(user, peer_map).await;
        info!("User {} disconnected", user.username);
    }
}
