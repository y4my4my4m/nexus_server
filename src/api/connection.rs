// server/src/api/connection.rs

use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;
use futures::{SinkExt, StreamExt};
use tracing::error;
use common::{ClientMessage, ServerMessage};

use crate::db;
use regex; // Add regex import

// Peer and PeerMap types for session management
pub struct Peer {
    pub user_id: Option<Uuid>,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
}

pub type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;

// Helper: Get all channel IDs a user is a member of
async fn get_user_channel_ids(user_id: Uuid) -> Vec<Uuid> {
    let servers = match db::servers::db_get_user_servers(user_id).await {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let mut channel_ids = Vec::new();
    for server in servers {
        for channel in server.channels {
            if channel.userlist.contains(&user_id) {
                channel_ids.push(channel.id);
            }
        }
    }
    channel_ids
}

// Helper: Broadcast a message to all users in the same channels as user_id
async fn broadcast_to_user_channels(peer_map: &PeerMap, user_id: Uuid, msg: &ServerMessage) {
    let channel_ids = get_user_channel_ids(user_id).await;
    let peers = peer_map.lock().await;
    let mut sent = std::collections::HashSet::new();
    for peer in peers.values() {
        if let Some(uid) = peer.user_id {
            // If this peer shares any channel with user_id
            let peer_channels = get_user_channel_ids(uid).await;
            if channel_ids.iter().any(|cid| peer_channels.contains(cid)) && sent.insert(uid) {
                let _ = peer.tx.send(msg.clone());
            }
        }
    }
}

// Helper: Push latest notifications to a user if online
async fn push_notifications_if_online(peer_map: &PeerMap, user_id: Uuid) {
    let peers = peer_map.lock().await;
    for peer in peers.values() {
        if peer.user_id == Some(user_id) {
            if let Ok((notifications, history_complete)) = db::notifications::db_get_notifications(user_id, None).await {
                let _ = peer.tx.send(ServerMessage::Notifications { notifications, history_complete });
            }
        }
    }
}

// Helper: Broadcast a message to all users in a channel
async fn broadcast_to_channel_users(
    peer_map: &PeerMap,
    channel_user_ids: &[Uuid],
    msg: &ServerMessage,
) {
    let peers = peer_map.lock().await;
    for peer in peers.values() {
        if let Some(uid) = peer.user_id {
            if channel_user_ids.contains(&uid) {
                let _ = peer.tx.send(msg.clone());
            }
        }
    }
}

// Helper: Handle mentions in a channel message (insert notification, send real-time event if online)
async fn handle_channel_mentions(
    peer_map: &PeerMap,
    channel_userlist: &[common::User],
    author_profile: &common::User,
    msg_id: Uuid,
    content: &str,
) {
    let mention_re = regex::Regex::new(r"@([a-zA-Z0-9_]+)").unwrap();
    let mentioned: Vec<String> = mention_re
        .captures_iter(content)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect();
    if !mentioned.is_empty() {
        // First, insert notifications for mentioned users who are in the channel
        for username in &mentioned {
            if let Some(user) = channel_userlist.iter().find(|u| u.username == *username) {
                let _ = db::notifications::db_insert_notification(
                    user.id,
                    "Mention",
                    msg_id,
                    Some(format!("From: {}", author_profile.username)),
                ).await;
            }
        }
        // Then, send real-time mention notifications to online users (like old_main.rs)
        let peers = peer_map.lock().await;
        for username in mentioned {
            // Find the mentioned user among all online peers (like old_main.rs does)
            for peer in peers.values() {
                if let Some(uid) = peer.user_id {
                    if let Ok(profile) = db::users::db_get_user_by_id(uid).await {
                        if profile.username == username {
                            let _ = peer.tx.send(ServerMessage::MentionNotification {
                                from: author_profile.clone(),
                                content: content.to_string(),
                            });
                            break; // Found the user, stop searching
                        }
                    }
                }
            }
        }
    }
}

// Main connection handler (moved from main.rs)
pub async fn handle_connection(
    stream: TcpStream,
    peer_map: PeerMap,
) {
    let peer_id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let user_id: Option<Uuid>;

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
    let mut sink = sink; // make sink mutable for both message handling and rx loop
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
                                    let result = db::users::db_register_user(&username, &password, "Reset", "User").await;
                                    let response = match result {
                                        Ok(profile) => ServerMessage::AuthSuccess(common::User {
                                            id: profile.id,
                                            username: profile.username,
                                            color: profile.color,
                                            role: profile.role,
                                            profile_pic: profile.profile_pic,
                                            cover_banner: profile.cover_banner,
                                            status: common::UserStatus::Connected,
                                        }),
                                        Err(e) => ServerMessage::AuthFailure(e),
                                    };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::Login { username, password } => {
                                    let result = db::users::db_login_user(&username, &password).await;
                                    match &result {
                                        Ok(profile) => {
                                            // Set user_id in peer map
                                            let mut peers = peer_map_task.lock().await;
                                            if let Some(peer) = peers.get_mut(&peer_id) {
                                                peer.user_id = Some(profile.id);
                                            }
                                            let user = common::User {
                                                id: profile.id,
                                                username: profile.username.clone(),
                                                color: profile.color,
                                                role: profile.role,
                                                profile_pic: profile.profile_pic.clone(),
                                                cover_banner: profile.cover_banner.clone(),
                                                status: common::UserStatus::Connected,
                                            };
                                            current_user = Some(user.clone());
                                            let response = ServerMessage::AuthSuccess(user.clone());
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            drop(peers); // Drop lock before broadcast
                                            broadcast_to_user_channels(&peer_map_task, user.id, &ServerMessage::UserJoined(user)).await;
                                        }
                                        Err(e) => {
                                            let response = ServerMessage::AuthFailure(e.clone());
                                            let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                        }
                                    }
                                }
                                ClientMessage::Logout => {
                                    let mut peers = peer_map_task.lock().await;
                                    if let Some(peer) = peers.get_mut(&peer_id) {
                                        peer.user_id = None;
                                    }
                                    let user_opt = current_user.clone();
                                    current_user = None;
                                    drop(peers); // Drop lock before broadcast
                                    if let Some(user) = user_opt {
                                        broadcast_to_user_channels(&peer_map_task, user.id, &ServerMessage::UserLeft(user.id)).await;
                                    }
                                }
                                ClientMessage::GetForums => {
                                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                                    let response = ServerMessage::Forums(forums);
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetServers => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let servers = db::servers::db_get_user_servers(user_id).await.unwrap_or_default();
                                        let response = ServerMessage::Servers(servers);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    } else {
                                        let response = ServerMessage::Notification("Not logged in".to_string(), true);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::GetChannelMessages { channel_id, before } => {
                                    let (messages, history_complete) = db::channels::db_get_channel_messages(channel_id, before).await.unwrap_or_default();
                                    let response = ServerMessage::ChannelMessages { channel_id, messages, history_complete };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetChannelUserList { channel_id } => {
                                    let mut users = db::channels::db_get_channel_user_list(channel_id).await.unwrap_or_default();
                                    // Update status for online users, set Offline otherwise
                                    let peers = peer_map_task.lock().await;
                                    for user in users.iter_mut() {
                                        if peers.values().any(|p| p.user_id == Some(user.id)) {
                                            user.status = common::UserStatus::Connected;
                                        } else {
                                            user.status = common::UserStatus::Offline;
                                        }
                                    }
                                    let response = ServerMessage::ChannelUserList { channel_id, users };
                                    let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                }
                                ClientMessage::GetDMUserList => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let users = db::messages::db_get_dm_user_list(user_id).await.unwrap_or_default();
                                        let response = ServerMessage::DMUserList(users);
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::GetDirectMessages { user_id, before } => {
                                    if let Some(my_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let (messages, history_complete) = db::messages::db_get_direct_messages(my_id, user_id, before).await.unwrap_or_default();
                                        let response = ServerMessage::DirectMessages { user_id, messages, history_complete };
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::SendDirectMessage { to, content } => {
                                    if let Some(user) = &current_user {
                                        let timestamp = chrono::Utc::now().timestamp();
                                        let dm_id = db::messages::db_store_direct_message(user.id, to, &content, timestamp).await;
                                        let dm_id = match dm_id {
                                            Ok(id) => id,
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to send DM: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                                continue;
                                            }
                                        };
                                        // Fetch author info for the message
                                        if let Ok(author_profile) = db::users::db_get_user_by_id(user.id).await {
                                            let dm = common::DirectMessage {
                                                id: dm_id,
                                                from: user.id,
                                                to,
                                                timestamp,
                                                content: content.clone(),
                                                author_username: author_profile.username,
                                                author_color: author_profile.color,
                                                author_profile_pic: author_profile.profile_pic,
                                            };
                                            // Create notification for recipient, referencing the DM UUID (using from_user.username like old code)
                                            let _ = db::notifications::db_insert_notification(
                                                to,
                                                "DM",
                                                dm_id,
                                                Some(format!("From: {}", user.username))
                                            ).await;
                                            // Send DM to recipient and sender if online
                                            let peers = peer_map_task.lock().await;
                                            for peer in peers.values() {
                                                if let Some(uid) = peer.user_id {
                                                    if uid == to || uid == user.id {
                                                        let _ = peer.tx.send(ServerMessage::DirectMessage(dm.clone()));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                ClientMessage::SendChannelMessage { channel_id, content } => {
                                    if let Some(user) = &current_user {
                                        let timestamp = chrono::Utc::now().timestamp();
                                        match db::channels::db_create_channel_message(channel_id, user.id, timestamp, &content).await {
                                            Ok(msg_id) => {
                                                // Fetch author info for the message
                                                if let Ok(author_profile) = db::users::db_get_user_by_id(user.id).await {
                                                    let channel_msg = common::ChannelMessage {
                                                        id: msg_id,
                                                        channel_id,
                                                        sent_by: user.id,
                                                        timestamp,
                                                        content: content.clone(),
                                                        author_username: author_profile.username.clone(),
                                                        author_color: author_profile.color,
                                                        author_profile_pic: author_profile.profile_pic.clone(),
                                                    };
                                                    // Fetch channel userlist from DB ONCE
                                                    let channel_userlist = db::channels::db_get_channel_user_list(channel_id).await.unwrap_or_default();
                                                    let channel_user_ids: Vec<Uuid> = channel_userlist.iter().map(|u| u.id).collect();
                                                    // Broadcast to all users in the channel
                                                    broadcast_to_channel_users(&peer_map_task, &channel_user_ids, &ServerMessage::NewChannelMessage(channel_msg.clone())).await;
                                                    // Handle mentions (insert notification, send real-time event if online)
                                                    handle_channel_mentions(&peer_map_task, &channel_userlist, user, msg_id, &content).await;
                                                }
                                            }
                                            Err(e) => {
                                                let response = ServerMessage::Notification(format!("Failed to send channel message: {}", e), true);
                                                let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::GetNotifications { before } => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let (notifications, history_complete) = db::notifications::db_get_notifications(user_id, before).await.unwrap_or_default();
                                        let response = ServerMessage::Notifications { notifications, history_complete };
                                        let _ = sink.send(bincode::serialize(&response).unwrap().into()).await;
                                    }
                                }
                                ClientMessage::MarkNotificationRead { notification_id } => {
                                    let _ = db::notifications::db_mark_notification_read(notification_id).await;
                                }
                                ClientMessage::UpdatePassword(new_password) => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let _ = db::users::db_update_user_password(user_id, &new_password).await;
                                    }
                                }
                                ClientMessage::UpdateProfile { bio, url1, url2, url3, location, profile_pic, cover_banner } => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let _ = db::users::db_update_user_profile(user_id, bio, url1, url2, url3, location, profile_pic, cover_banner).await;
                                        // Fetch updated user
                                        if let Ok(profile) = db::users::db_get_user_by_id(user_id).await {
                                            let user = common::User {
                                                id: profile.id,
                                                username: profile.username,
                                                color: profile.color,
                                                role: profile.role,
                                                profile_pic: profile.profile_pic.clone(),
                                                cover_banner: profile.cover_banner.clone(),
                                                status: common::UserStatus::Connected,
                                            };
                                            broadcast_to_user_channels(&peer_map_task, user.id, &ServerMessage::UserUpdated(user.clone())).await;
                                            // Also broadcast updated ChannelUserList for each channel the user is in
                                            let channel_ids = get_user_channel_ids(user.id).await;
                                            for channel_id in channel_ids {
                                                let users = db::channels::db_get_channel_user_list(channel_id).await.unwrap_or_default();
                                                broadcast_to_channel_users(&peer_map_task, &users.iter().map(|u| u.id).collect::<Vec<_>>(), &ServerMessage::ChannelUserList { channel_id, users }).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::UpdateColor(color) => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        let color_str = match color.0 {
                                            ratatui::style::Color::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
                                            other => format!("{:?}", other),
                                        };
                                        let _ = db::users::db_update_user_color(user_id, &color_str).await;
                                        // Fetch updated user
                                        if let Ok(profile) = db::users::db_get_user_by_id(user_id).await {
                                            let user = common::User {
                                                id: profile.id,
                                                username: profile.username,
                                                color: profile.color,
                                                role: profile.role,
                                                profile_pic: profile.profile_pic.clone(),
                                                cover_banner: profile.cover_banner.clone(),
                                                status: common::UserStatus::Connected,
                                            };
                                            broadcast_to_user_channels(&peer_map_task, user.id, &ServerMessage::UserUpdated(user.clone())).await;
                                            // Also broadcast updated ChannelUserList for each channel the user is in
                                            let channel_ids = get_user_channel_ids(user.id).await;
                                            for channel_id in channel_ids {
                                                let users = db::channels::db_get_channel_user_list(channel_id).await.unwrap_or_default();
                                                broadcast_to_channel_users(&peer_map_task, &users.iter().map(|u| u.id).collect::<Vec<_>>(), &ServerMessage::ChannelUserList { channel_id, users }).await;
                                            }
                                        }
                                    }
                                }
                                ClientMessage::CreatePost { thread_id, content } => {
                                    if let Some(user_id) = peer_map_task.lock().await.get(&peer_id).and_then(|p| p.user_id) {
                                        match db::forums::db_create_post(thread_id, user_id, &content).await {
                                            Ok(_) => {
                                                // After creating the post, fetch the updated forums and send to client
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
                                ClientMessage::GetProfile { user_id } => {
                                    match db::users::db_get_user_profile(user_id).await {
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
                                _ => todo!(),
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
        drop(peers); // Drop lock before broadcast
        if let Some(user_id) = user_id_opt {
            // Broadcast UserLeft to all users in same channels
            broadcast_to_user_channels(&peer_map_task, user_id, &ServerMessage::UserLeft(user_id)).await;
            // Also broadcast updated ChannelUserList for each channel the user was in
            let channel_ids = get_user_channel_ids(user_id).await;
            for channel_id in channel_ids {
                let users = db::channels::db_get_channel_user_list(channel_id).await.unwrap_or_default();
                broadcast_to_channel_users(&peer_map_task, &users.iter().map(|u| u.id).collect::<Vec<_>>(), &ServerMessage::ChannelUserList { channel_id, users }).await;
            }
        }
    });
}
