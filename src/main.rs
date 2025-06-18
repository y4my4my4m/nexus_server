// server/src/main.rs

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use common::{
    ChatMessage, ClientMessage, Forum, Post, ServerMessage, Thread, User,
    UserProfile, UserRole, UserStatus,
};
use futures::{SinkExt, StreamExt};
use ratatui::style::Color;
use rusqlite::{params, Connection, Result as SqlResult};
use std::{collections::HashMap, env, error::Error, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
};
use tokio::task;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info};
use uuid::Uuid;

const DB_PATH: &str = "cyberpunk_bbs.db";

struct Peer {
    user_id: Option<Uuid>,
    tx: mpsc::UnboundedSender<ServerMessage>,
}

type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let listener = TcpListener::bind(&addr).await?;
    info!("Server listening on: {}", addr);

    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));

    // Initialize the database
    let _conn = init_db()?;
    // Ensure default server and channels exist
    ensure_default_server_exists().await.map_err(|e| format!("Failed to create default server: {}", e))?;

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle_connection(
            stream,
            peer_map.clone(),
        ));
    }
}

// *** FIX for Argon2 Error ***
fn hash_password(password: &str) -> Result<String, Box<dyn Error>> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    // Use map_err to convert the specific error type to a generic one
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string().into())
}

fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .and_then(|parsed_hash| Argon2::default().verify_password(password.as_bytes(), &parsed_hash))
        .is_ok()
}

async fn handle_connection(
    stream: TcpStream,
    peer_map: PeerMap,
) {
    let conn_id = Uuid::new_v4();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
    peer_map
        .lock()
        .await
        .insert(conn_id, Peer { user_id: None, tx });

    let (mut sink, mut stream) = Framed::new(stream, LengthDelimitedCodec::new()).split();

    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink
                .send(bincode::serialize(&msg).unwrap().into())
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let mut current_user: Option<User> = None;

    while let Some(Ok(data)) = stream.next().await {
        let msg: ClientMessage = match bincode::deserialize(&data) {
            Ok(m) => m,
            Err(e) => {
                error!("Deserialization error: {}", e);
                continue;
            }
        };
        info!("SERVER RX: Received message from client {:?}: {:?}", conn_id, msg);

        if let Some(ref user) = current_user {
            match msg {
                ClientMessage::Logout => {
                    // Broadcast UserLeft before removing
                    if let Some(ref user) = current_user {
                        broadcast(&peer_map, &ServerMessage::UserLeft(user.id)).await;
                    }
                    current_user = None;
                    peer_map.lock().await.get_mut(&conn_id).unwrap().user_id = None;
                }
                ClientMessage::UpdatePassword(new_password) => {
                    match db_update_user_password(user.id, &new_password).await {
                        Ok(()) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::Notification(
                                "Password updated.".into(),
                                false,
                            )).unwrap();
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::Notification(
                                format!("Password update failed: {}", e),
                                true,
                            )).unwrap();
                        }
                    }
                }
                ClientMessage::UpdateColor(serializable_color) => {
                    let color_str = format!("{:?}", serializable_color.0); // Store as string
                    match db_update_user_color(user.id, &color_str).await {
                        Ok(()) => {
                            // Reload the user from DB and update current_user
                            match db_get_user_by_id(user.id).await {
                                Ok(updated_user) => {
                                    current_user = Some(User {
                                        id: updated_user.id,
                                        username: updated_user.username.clone(),
                                        color: updated_user.color,
                                        role: updated_user.role.clone(),
                                        profile_pic: updated_user.profile_pic.clone(),
                                        cover_banner: updated_user.cover_banner.clone(),
                                        status: UserStatus::Connected, // <-- Set status here
                                    });
                                    let peers = peer_map.lock().await;
                                    let tx = &peers.get(&conn_id).unwrap().tx;
                                    // Notify client with updated user info
                                    tx.send(ServerMessage::Notification("Color updated.".into(), false)).unwrap();
                                }
                                Err(e) => {
                                    let peers = peer_map.lock().await;
                                    let tx = &peers.get(&conn_id).unwrap().tx;
                                    tx.send(ServerMessage::Notification(
                                        format!("Color updated, but failed to reload user: {}", e),
                                        true,
                                    )).unwrap();
                                }
                            }
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::Notification(
                                format!("Color update failed: {}", e),
                                true,
                            )).unwrap();
                        }
                    }
                }
                ClientMessage::GetForums => {
                    match db_get_forums().await {
                        Ok(forums) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::Forums(forums)).unwrap();
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::Notification(format!("Failed to load forums: {}", e), true)).unwrap();
                        }
                    }
                }
                ClientMessage::SendDirectMessage { to, content } => {
                    // Store DM in DB
                    let from_user = user.clone();
                    let to_user_id = to;
                    let now = chrono::Utc::now().timestamp();
                    let _ = db_store_direct_message(user.id, to_user_id, &content, now).await;
                    // Send DM to recipient if online
                    let peers = peer_map.lock().await;
                    for peer in peers.values() {
                        if let Some(uid) = peer.user_id {
                            if uid == to_user_id {
                                let _ = peer.tx.send(ServerMessage::DirectMessage {
                                    from: from_user.clone(),
                                    content: content.clone(),
                                });
                            }
                        }
                    }
                }
                ClientMessage::CreateThread {
                    forum_id,
                    title,
                    content,
                } => {
                    match db_create_thread(forum_id, &title, user.id, &content).await {
                        Ok(()) => {
                            // Send updated forums to all
                            match db_get_forums().await {
                                Ok(forums) => broadcast(&peer_map, &ServerMessage::Forums(forums)).await,
                                Err(e) => eprintln!("[ERROR] Failed to reload forums after thread creation: {}", e),
                            }
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::Notification(format!("Failed to create thread: {}", e), true)).unwrap();
                        }
                    }
                }
                ClientMessage::CreatePost { thread_id, content } => {
                    match db_create_post(thread_id, user.id, &content).await {
                        Ok(()) => {
                            // Send updated forums to all
                            match db_get_forums().await {
                                Ok(forums) => broadcast(&peer_map, &ServerMessage::Forums(forums)).await,
                                Err(e) => eprintln!("[ERROR] Failed to reload forums after post creation: {}", e),
                            }
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::Notification(format!("Failed to create post: {}", e), true)).unwrap();
                        }
                    }
                }
                ClientMessage::GetUserList => {
                    let peers = peer_map.lock().await;
                    let mut users = Vec::new();
                    for peer in peers.values() {
                        if let Some(uid) = peer.user_id {
                            if let Ok(_profile) = db_get_user_by_id(uid).await {
                                users.push(User {
                                    id: _profile.id,
                                    username: _profile.username,
                                    color: _profile.color,
                                    role: _profile.role,
                                    profile_pic: _profile.profile_pic.clone(),
                                    cover_banner: _profile.cover_banner.clone(),
                                    status: UserStatus::Connected, // <-- Set status here
                                });
                            }
                        }
                    }
                    let tx = &peers.get(&conn_id).unwrap().tx;
                    tx.send(ServerMessage::UserList(users)).unwrap();
                },
                ClientMessage::UpdateProfile { bio, url1, url2, url3, location, profile_pic, cover_banner } => {
                    match db_update_user_profile(user.id, bio, url1, url2, url3, location, profile_pic, cover_banner).await {
                        Ok(()) => {
                            if let Ok(profile) = db_get_user_profile(user.id).await {
                                let peers = peer_map.lock().await;
                                let tx = &peers.get(&conn_id).unwrap().tx;
                                let _ = tx.send(ServerMessage::Profile(profile.clone()));
                                // Broadcast updated User to all clients
                                if let Ok(updated_user) = db_get_user_by_id(user.id).await {
                                    let user_struct = User {
                                        id: updated_user.id,
                                        username: updated_user.username,
                                        color: updated_user.color,
                                        role: updated_user.role,
                                        profile_pic: updated_user.profile_pic.clone(),
                                        cover_banner: updated_user.cover_banner.clone(),
                                        status: UserStatus::Connected, // <-- Set status here
                                    };
                                    for peer in peers.values() {
                                        let _ = peer.tx.send(ServerMessage::UserUpdated(user_struct.clone()));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            let _ = tx.send(ServerMessage::Notification(format!("Profile update failed: {}", e), true));
                        }
                    }
                }
                ClientMessage::GetProfile { user_id } => {
                    match db_get_user_profile(user_id).await {
                        Ok(profile) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            let _ = tx.send(ServerMessage::Profile(profile));
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            let _ = tx.send(ServerMessage::Notification(format!("Profile fetch failed: {}", e), true));
                        }
                    }
                }
                ClientMessage::GetServers => {
                    let servers = db_get_user_servers(user.id).await;
                    let peers = peer_map.lock().await;
                    let tx = &peers.get(&conn_id).unwrap().tx;
                    match servers {
                        Ok(servers) => {
                            let _ = tx.send(ServerMessage::Servers(servers));
                        }
                        Err(e) => {
                            let _ = tx.send(ServerMessage::Notification(format!("Failed to load servers: {}", e), true));
                        }
                    }
                }
                ClientMessage::SendChannelMessage { channel_id, content } => {
                    let now = chrono::Utc::now().timestamp();
                    match db_create_channel_message(channel_id, user.id, now, &content).await {
                        Ok(msg_id) => {
                            // Fetch author info for the message
                            let author_profile = db_get_user_by_id(user.id).await.unwrap();
                            let channel_msg = ChannelMessage {
                                id: msg_id,
                                channel_id,
                                sent_by: user.id,
                                timestamp: now,
                                content: content.clone(),
                                author_username: author_profile.username,
                                author_color: author_profile.color,
                                author_profile_pic: author_profile.profile_pic,
                            };
                            // Broadcast only to users in the channel
                            let peers = peer_map.lock().await;
                            // Fetch channel userlist from DB
                            let channel_userlist = {
                                let conn = Connection::open(DB_PATH).unwrap();
                                let mut stmt = conn.prepare("SELECT user_id FROM channel_users WHERE channel_id = ?1").unwrap();
                                stmt.query_map(params![channel_id.to_string()], |row| row.get::<_, String>(0))
                                    .unwrap()
                                    .map(|r| Uuid::parse_str(&r.unwrap()).unwrap())
                                    .collect::<Vec<Uuid>>()
                            };
                            for (peer_id, peer) in peers.iter() {
                                if let Some(uid) = peer.user_id {
                                    if channel_userlist.contains(&uid) {
                                        let _ = peer.tx.send(ServerMessage::NewChannelMessage(channel_msg.clone()));
                                    }
                                }
                            }
                            // --- Mention logic ---
                            let mentioned = extract_mentions(&content);
                            if !mentioned.is_empty() {
                                for username in mentioned {
                                    if let Some((_, peer)) = peers.iter().find(|(_, p)| {
                                        if let Some(uid) = p.user_id {
                                            if let Ok(profile) = futures::executor::block_on(db_get_user_by_id(uid)) {
                                                profile.username == username
                                            } else { false }
                                        } else { false }
                                    }) {
                                        if let Some(uid) = peer.user_id {
                                            if let Ok(_profile) = db_get_user_by_id(uid).await {
                                                let from_user = user.clone();
                                                let tx = &peer.tx;
                                                let _ = tx.send(ServerMessage::MentionNotification {
                                                    from: from_user,
                                                    content: content.clone(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            let _ = tx.send(ServerMessage::Notification(format!("Failed to send channel message: {}", e), true));
                        }
                    }
                }
                ClientMessage::GetChannelMessages { channel_id } => {
                    // Fetch last 50 messages for the channel
                    let messages = {
                        let conn = Connection::open(DB_PATH).unwrap();
                        let mut stmt = conn.prepare("SELECT id, sent_by, timestamp, content FROM channel_messages WHERE channel_id = ?1 ORDER BY timestamp DESC LIMIT 50").unwrap();
                        let rows = stmt.query_map(params![channel_id.to_string()], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?))
                        }).unwrap();
                        let mut msgs = Vec::new();
                        for row in rows {
                            let (id, sent_by, timestamp, content) = row.unwrap();
                            let sent_by_uuid = Uuid::parse_str(&sent_by).unwrap();
                            let author_profile = futures::executor::block_on(db_get_user_by_id(sent_by_uuid)).unwrap();
                            msgs.push(ChannelMessage {
                                id: Uuid::parse_str(&id).unwrap(),
                                channel_id,
                                sent_by: sent_by_uuid,
                                timestamp,
                                content,
                                author_username: author_profile.username,
                                author_color: author_profile.color,
                                author_profile_pic: author_profile.profile_pic,
                            });
                        }
                        msgs.reverse(); // Oldest first
                        msgs
                    };
                    let peers = peer_map.lock().await;
                    let tx = &peers.get(&conn_id).unwrap().tx;
                    let _ = tx.send(ServerMessage::ChannelMessages { channel_id, messages });
                }
                ClientMessage::GetChannelUserList { channel_id } => {
                    // Fetch all users for the channel in a blocking task
                    let channel_id_str = channel_id.to_string();
                    let user_ids: Vec<Uuid> = tokio::task::spawn_blocking(move || {
                        let conn = Connection::open(DB_PATH).unwrap();
                        let mut stmt = conn.prepare("SELECT user_id FROM channel_users WHERE channel_id = ?1").unwrap();
                        stmt.query_map(params![channel_id_str], |row| row.get::<_, String>(0))
                            .unwrap()
                            .map(|r| Uuid::parse_str(&r.unwrap()).unwrap())
                            .collect::<Vec<Uuid>>()
                    }).await.unwrap();
                    let peers = peer_map.lock().await;
                    let mut users = Vec::new();
                    for uid in user_ids {
                        let status = if peers.values().any(|p| p.user_id == Some(uid)) {
                            common::UserStatus::Connected
                        } else {
                            common::UserStatus::Offline
                        };
                        if let Ok(profile) = db_get_user_by_id(uid).await {
                            users.push(common::User {
                                id: profile.id,
                                username: profile.username,
                                color: profile.color,
                                role: profile.role,
                                profile_pic: profile.profile_pic.clone(),
                                cover_banner: profile.cover_banner.clone(),
                                status,
                            });
                        }
                    }
                    let tx = &peers.get(&conn_id).unwrap().tx;
                    let _ = tx.send(ServerMessage::ChannelUserList { channel_id, users });
                }
                _ => {} // Ignore other messages when logged in
            }
        } else { // Not logged in
            match msg {
                ClientMessage::Register { username, password } => {
                    // Use SQLite for registration
                    let is_first_user = {
                        let conn = Connection::open(DB_PATH).unwrap();
                        let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0)).unwrap();
                        count == 0
                    };
                    let role = if is_first_user { "Admin" } else { "User" };
                    match db_register_user(&username, &password, "Green", role).await {
                        Ok(profile) => {
                            // Add user to default server and all its channels
                            let default_server_id = {
                                let conn = Connection::open(DB_PATH).unwrap();
                                conn.query_row("SELECT id FROM servers LIMIT 1", [], |row| row.get::<_, String>(0)).ok()
                            };
                            if let Some(server_id_str) = default_server_id {
                                let server_id = Uuid::parse_str(&server_id_str).unwrap();
                                let user_id = profile.id;
                                let _ = tokio::task::spawn_blocking(move || {
                                    let conn = Connection::open(DB_PATH).unwrap();
                                    conn.execute(
                                        "INSERT OR IGNORE INTO server_users (server_id, user_id) VALUES (?1, ?2)",
                                        params![server_id.to_string(), user_id.to_string()]
                                    ).ok();
                                    let mut stmt = conn.prepare("SELECT id FROM channels WHERE server_id = ?1").unwrap();
                                    let channel_ids: Vec<String> = stmt.query_map(params![server_id.to_string()], |row| row.get::<_, String>(0))
                                        .unwrap()
                                        .map(|r| r.unwrap())
                                        .collect();
                                    for chan_id in channel_ids {
                                        conn.execute(
                                            "INSERT OR IGNORE INTO channel_users (channel_id, user_id) VALUES (?1, ?2)",
                                            params![chan_id, user_id.to_string()]
                                        ).ok();
                                    }
                                }).await;
                            }
                            match db_get_user_by_id(profile.id).await {
                                Ok(full_profile) => {
                                    let user_data = User {
                                        id: full_profile.id,
                                        username: full_profile.username.clone(),
                                        color: full_profile.color,
                                        role: full_profile.role.clone(),
                                        profile_pic: full_profile.profile_pic.clone(),
                                        cover_banner: full_profile.cover_banner.clone(),
                                        status: UserStatus::Connected, // <-- Set status here
                                    };
                                    current_user = Some(user_data.clone());
                                    peer_map.lock().await.get_mut(&conn_id).unwrap().user_id = Some(user_data.id);
                                    let peers = peer_map.lock().await;
                                    let tx = &peers.get(&conn_id).unwrap().tx;
                                    tx.send(ServerMessage::AuthSuccess(user_data.clone())).unwrap();
                                    // Broadcast UserJoined
                                    drop(peers); // unlock before broadcast
                                    broadcast(&peer_map, &ServerMessage::UserJoined(user_data)).await;
                                }
                                Err(e) => {
                                    let peers = peer_map.lock().await;
                                    let tx = &peers.get(&conn_id).unwrap().tx;
                                    tx.send(ServerMessage::AuthFailure(e)).unwrap();
                                }
                            }
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::AuthFailure(e)).unwrap();
                        }
                    }
                }
                ClientMessage::Login { username, password } => {
                    match db_login_user(&username, &password).await {
                        Ok(profile) => {
                            match db_get_user_by_id(profile.id).await {
                                Ok(full_profile) => {
                                    let user_data = User {
                                        id: full_profile.id,
                                        username: full_profile.username.clone(),
                                        color: full_profile.color,
                                        role: full_profile.role.clone(),
                                        profile_pic: full_profile.profile_pic.clone(),
                                        cover_banner: full_profile.cover_banner.clone(),
                                        status: UserStatus::Connected, // <-- Set status here
                                    };
                                    current_user = Some(user_data.clone());
                                    let tx = {
                                        let mut peers = peer_map.lock().await;
                                        if let Some(peer) = peers.get_mut(&conn_id) {
                                            peer.user_id = Some(user_data.id);
                                            peer.tx.clone()
                                        } else {
                                            continue;
                                        }
                                    };
                                    tx.send(ServerMessage::AuthSuccess(user_data.clone())).unwrap();
                                    // Broadcast UserJoined
                                    broadcast(&peer_map, &ServerMessage::UserJoined(user_data)).await;
                                }
                                Err(e) => {
                                    let peers = peer_map.lock().await;
                                    let tx = &peers.get(&conn_id).unwrap().tx;
                                    tx.send(ServerMessage::AuthFailure(e)).unwrap();
                                }
                            }
                        }
                        Err(e) => {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::AuthFailure(e)).unwrap();
                        }
                    }
                }
                _ => {} // Ignore authenticated messages
            }
        }
    }
    peer_map.lock().await.remove(&conn_id);
    if let Some(user) = current_user {
        // Broadcast UserLeft on disconnect only to users who share a channel
        let peers = peer_map.lock().await;
        // Find all users who share a channel with this user
        let shared_channel_user_ids: Vec<Uuid> = {
            let conn = Connection::open(DB_PATH).unwrap();
            let mut stmt = conn.prepare("SELECT channel_id FROM channel_users WHERE user_id = ?1").unwrap();
            let user_channels: Vec<String> = stmt.query_map(params![user.id.to_string()], |row| row.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect();
            let mut user_ids = Vec::new();
            for chan_id in user_channels {
                let mut stmt2 = conn.prepare("SELECT user_id FROM channel_users WHERE channel_id = ?1").unwrap();
                let ids = stmt2.query_map(params![chan_id], |row| row.get::<_, String>(0)).unwrap().map(|r| Uuid::parse_str(&r.unwrap()).unwrap());
                user_ids.extend(ids);
            }
            user_ids.sort();
            user_ids.dedup();
            user_ids
        };
        for peer in peers.values() {
            if let Some(uid) = peer.user_id {
                if shared_channel_user_ids.contains(&uid) {
                    let _ = peer.tx.send(ServerMessage::UserLeft(user.id));
                }
            }
        }
        info!("{} disconnected.", user.username);
    }
}

async fn broadcast(peer_map: &PeerMap, msg: &ServerMessage) {
    let peers = peer_map.lock().await;
    for peer in peers.values() {
        if peer.user_id.is_some() {
            let _ = peer.tx.send(msg.clone());
        }
    }
}

fn init_db() -> SqlResult<Connection> {
    let conn = Connection::open(DB_PATH)?;
    // Users
    conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            color TEXT NOT NULL,
            role TEXT NOT NULL
        )",
        [],
    )?;
    // --- MIGRATION: Add missing profile columns if not present ---
    let columns = [
        ("bio", "TEXT"),
        ("url1", "TEXT"),
        ("url2", "TEXT"),
        ("url3", "TEXT"),
        ("location", "TEXT"),
        ("profile_pic", "TEXT"),
        ("cover_banner", "TEXT"),
    ];
    for (col, ty) in columns.iter() {
        let sql = format!("ALTER TABLE users ADD COLUMN {} {}", col, ty);
        let res = conn.execute(&sql, []);
        if let Err(e) = res {
            if !e.to_string().contains("duplicate column name") {
                return Err(e);
            }
        }
    }
    // Forums
    conn.execute(
        "CREATE TABLE IF NOT EXISTS forums (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL
        )",
        [],
    )?;
    // Threads
    conn.execute(
        "CREATE TABLE IF NOT EXISTS threads (
            id TEXT PRIMARY KEY,
            forum_id TEXT NOT NULL,
            title TEXT NOT NULL,
            author_id TEXT NOT NULL,
            FOREIGN KEY(forum_id) REFERENCES forums(id),
            FOREIGN KEY(author_id) REFERENCES users(id)
        )",
        [],
    )?;
    // Posts
    conn.execute(
        "CREATE TABLE IF NOT EXISTS posts (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            author_id TEXT NOT NULL,
            content TEXT NOT NULL,
            FOREIGN KEY(thread_id) REFERENCES threads(id),
            FOREIGN KEY(author_id) REFERENCES users(id)
        )",
        [],
    )?;
    // Direct Messages
    conn.execute(
        "CREATE TABLE IF NOT EXISTS direct_messages (
            id TEXT PRIMARY KEY,
            from_user_id TEXT NOT NULL,
            to_user_id TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            FOREIGN KEY(from_user_id) REFERENCES users(id),
            FOREIGN KEY(to_user_id) REFERENCES users(id)
        )",
        [],
    )?;
    // --- SERVERS/CHANNELS/CHANNEL_MESSAGES ---
    conn.execute(
        "CREATE TABLE IF NOT EXISTS servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            public INTEGER NOT NULL,
            invite_code TEXT,
            icon TEXT,
            banner TEXT,
            owner TEXT NOT NULL,
            FOREIGN KEY(owner) REFERENCES users(id)
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS server_mods (
            server_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            PRIMARY KEY(server_id, user_id),
            FOREIGN KEY(server_id) REFERENCES servers(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS server_users (
            server_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            PRIMARY KEY(server_id, user_id),
            FOREIGN KEY(server_id) REFERENCES servers(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channels (
            id TEXT PRIMARY KEY,
            server_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            FOREIGN KEY(server_id) REFERENCES servers(id)
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channel_users (
            channel_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            PRIMARY KEY(channel_id, user_id),
            FOREIGN KEY(channel_id) REFERENCES channels(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channel_permissions (
            channel_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            can_read INTEGER NOT NULL,
            can_write INTEGER NOT NULL,
            PRIMARY KEY(channel_id, user_id),
            FOREIGN KEY(channel_id) REFERENCES channels(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channel_messages (
            id TEXT PRIMARY KEY,
            channel_id TEXT NOT NULL,
            sent_by TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            content TEXT NOT NULL,
            FOREIGN KEY(channel_id) REFERENCES channels(id),
            FOREIGN KEY(sent_by) REFERENCES users(id)
        )",
        [],
    )?;
    Ok(conn)
}

// --- SQLite User Management ---

async fn db_get_user_by_id(user_id: Uuid) -> Result<UserProfile, String> {
    let user_id = user_id.to_string();
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT id, username, password_hash, color, role, bio, url1, url2, url3, location, profile_pic, cover_banner FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
        let user = stmt.query_row(params![user_id], |row| {
            let id: String = row.get(0)?;
            let username: String = row.get(1)?;
            let hash: String = row.get(2)?;
            let color: String = row.get(3)?;
            let role: String = row.get(4)?;
            let bio: Option<String> = row.get(5)?;
            let url1: Option<String> = row.get(6)?;
            let url2: Option<String> = row.get(7)?;
            let url3: Option<String> = row.get(8)?;
            let location: Option<String> = row.get(9)?;
            let profile_pic: Option<String> = row.get(10)?;
            let cover_banner: Option<String> = row.get(11)?;
            Ok((id, username, hash, color, role, bio, url1, url2, url3, location, profile_pic, cover_banner))
        }).map_err(|_| "User not found".to_string())?;
        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: String::new(), // Do not send password hash to client
            color: parse_color(&user.3),
            role: match user.4.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: user.5,
            url1: user.6,
            url2: user.7,
            url3: user.8,
            location: user.9,
            profile_pic: user.10,
            cover_banner: user.11,
        })
    }).await.unwrap()
}

async fn db_register_user(username: &str, password: &str, color: &str, role: &str) -> Result<UserProfile, String> {
    let username = username.to_string();
    let password = password.to_string();
    let color = color.to_string();
    let role = role.to_string();
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        // Check if username exists
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM users WHERE username = ?1").map_err(|e| e.to_string())?;
        let exists: i64 = stmt.query_row(params![username], |row| row.get(0)).map_err(|e| e.to_string())?;
        if exists > 0 {
            return Err("Username taken".to_string());
        }
        let id = Uuid::new_v4();
        let hash = hash_password(&password).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO users (id, username, password_hash, color, role) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), username, hash, color, role],
        ).map_err(|e| e.to_string())?;
        Ok(UserProfile {
            id,
            username,
            hash,
            color: parse_color(&color),
            role: match role.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: None,
            url1: None,
            url2: None,
            url3: None,
            location: None,
            profile_pic: None,
            cover_banner: None,
        })
    }).await.unwrap()
}

async fn db_login_user(username: &str, password: &str) -> Result<UserProfile, String> {
    let username = username.to_string();
    let password = password.to_string();
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT id, username, password_hash, color, role FROM users WHERE username = ?1").map_err(|e| e.to_string())?;
        let user = stmt.query_row(params![username], |row| {
            let id: String = row.get(0)?;
            let username: String = row.get(1)?;
            let hash: String = row.get(2)?;
            let color: String = row.get(3)?;
            let role: String = row.get(4)?;
            Ok((id, username, hash, color, role))
        }).map_err(|_| "Invalid credentials".to_string())?;
        if !verify_password(&user.2, &password) {
            return Err("Invalid credentials".to_string());
        }
        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: String::new(), // Do not send password hash to client
            color: parse_color(&user.3),
            role: match user.4.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: None,
            url1: None,
            url2: None,
            url3: None,
            location: None,
            profile_pic: None,
            cover_banner: None,
        })
    }).await.unwrap()
}

async fn db_update_user_password(user_id: Uuid, new_password: &str) -> Result<(), String> {
    let user_id = user_id.to_string();
    let new_password = new_password.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let hash = hash_password(&new_password).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![hash, user_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

async fn db_update_user_color(user_id: Uuid, color: &str) -> Result<(), String> {
    let user_id = user_id.to_string();
    let color = color.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE users SET color = ?1 WHERE id = ?2",
            params![color, user_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

fn validate_profile_fields(
    bio: &Option<String>,
    url1: &Option<String>,
    url2: &Option<String>,
    url3: &Option<String>,
    location: &Option<String>,
    profile_pic: &Option<String>,
    cover_banner: &Option<String>,
) -> Result<(), String> {
    if let Some(bio) = bio {
        if bio.len() > 5000 {
            return Err("Bio must be at most 5000 characters.".to_string());
        }
    }
    for (i, url) in [url1, url2, url3].iter().enumerate() {
        if let Some(u) = url {
            if u.len() > 100 {
                return Err(format!("URL{} must be at most 100 characters.", i + 1));
            }
        }
    }
    if let Some(loc) = location {
        if loc.len() > 100 {
            return Err("Location must be at most 100 characters.".to_string());
        }
    }
    if let Some(pic) = profile_pic {
        if pic.len() > 1024 * 1024 {
            return Err("Profile picture must be at most 1MB (base64 or URL).".to_string());
        }
    }
    if let Some(banner) = cover_banner {
        if banner.len() > 1024 * 1024 {
            return Err("Cover banner must be at most 1MB (base64 or URL).".to_string());
        }
    }
    Ok(())
}

async fn db_update_user_profile(user_id: Uuid, bio: Option<String>, url1: Option<String>, url2: Option<String>, url3: Option<String>, location: Option<String>, profile_pic: Option<String>, cover_banner: Option<String>) -> Result<(), String> {
    validate_profile_fields(&bio, &url1, &url2, &url3, &location, &profile_pic, &cover_banner)?;
    let user_id = user_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut query = "UPDATE users SET ".to_string();
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
        let mut set_fields = Vec::new();
        if let Some(bio) = &bio { set_fields.push("bio = ?"); params.push(bio as &dyn rusqlite::ToSql); }
        if let Some(url1) = &url1 { set_fields.push("url1 = ?"); params.push(url1 as &dyn rusqlite::ToSql); }
        if let Some(url2) = &url2 { set_fields.push("url2 = ?"); params.push(url2 as &dyn rusqlite::ToSql); }
        if let Some(url3) = &url3 { set_fields.push("url3 = ?"); params.push(url3 as &dyn rusqlite::ToSql); }
        if let Some(location) = &location { set_fields.push("location = ?"); params.push(location as &dyn rusqlite::ToSql); }
        if let Some(profile_pic) = &profile_pic { set_fields.push("profile_pic = ?"); params.push(profile_pic as &dyn rusqlite::ToSql); }
        if let Some(cover_banner) = &cover_banner { set_fields.push("cover_banner = ?"); params.push(cover_banner as &dyn rusqlite::ToSql); }
        if set_fields.is_empty() {
            return Ok(());
        }
        query.push_str(&set_fields.join(", "));
        query.push_str(" WHERE id = ?");
        params.push(&user_id as &dyn rusqlite::ToSql);
        conn.execute(&query, &params[..]).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

async fn db_get_user_profile(user_id: Uuid) -> Result<UserProfile, String> {
    let user_id = user_id.to_string();
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT id, username, bio, url1, url2, url3, location, profile_pic, cover_banner FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
        let user = stmt.query_row(params![user_id], |row| {
            let id: String = row.get(0)?;
            let username: String = row.get(1)?;
            let bio: Option<String> = row.get(2)?;
            let url1: Option<String> = row.get(3)?;
            let url2: Option<String> = row.get(4)?;
            let url3: Option<String> = row.get(5)?;
            let location: Option<String> = row.get(6)?;
            let profile_pic: Option<String> = row.get(7)?;
            let cover_banner: Option<String> = row.get(8)?;
            Ok((id, username, bio, url1, url2, url3, location, profile_pic, cover_banner))
        }).map_err(|_| "User not found".to_string())?;
        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: "".to_string(), // Hash is not fetched for profile view
            color: Color::Reset, // Color is not fetched for profile view
            role: UserRole::User, // Role is not fetched for profile view
            bio: user.2,
            url1: user.3,
            url2: user.4,
            url3: user.5,
            location: user.6,
            profile_pic: user.7,
            cover_banner: user.8,
        })
    }).await.unwrap()
}

// --- SQLite Forum/Thread/Post Management ---

async fn db_get_forums() -> Result<Vec<Forum>, String> {
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    let mut forums = Vec::new();
    let mut stmt = conn.prepare("SELECT id, name, description FROM forums").map_err(|e| e.to_string())?;
    let forum_rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    }).map_err(|e| e.to_string())?;
    for forum_row in forum_rows {
        let (forum_id, name, description) = forum_row.map_err(|e| e.to_string())?;
        let forum_uuid = Uuid::parse_str(&forum_id).map_err(|e| e.to_string())?;
        // Get threads for this forum
        let mut thread_stmt = conn.prepare("SELECT id, title, author_id FROM threads WHERE forum_id = ?1").map_err(|e| e.to_string())?;
        let thread_rows = thread_stmt.query_map(params![forum_id.clone()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        }).map_err(|e| e.to_string())?;
        let mut threads = Vec::new();
        for thread_row in thread_rows {
            let (thread_id, title, author_id) = thread_row.map_err(|e| e.to_string())?;
            let thread_uuid = Uuid::parse_str(&thread_id).map_err(|e| e.to_string())?;
            // Get author user
            let mut user_stmt = conn.prepare("SELECT id, username, color, role FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
            let user_row = user_stmt.query_row(params![author_id.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            }).map_err(|e| e.to_string())?;
            let (user_id, username, color, role) = user_row;
            // Fetch author profile for profile_pic and cover_banner
            let author_profile = futures::executor::block_on(db_get_user_profile(Uuid::parse_str(&user_id).unwrap())).map_err(|e| e.to_string())?;
            let author = User {
                id: Uuid::parse_str(&user_id).unwrap(),
                username,
                color: parse_color(&color),
                role: match role.as_str() {
                    "Admin" => UserRole::Admin,
                    "Moderator" => UserRole::Moderator,
                    _ => UserRole::User,
                },
                profile_pic: author_profile.profile_pic.clone(),
                cover_banner: author_profile.cover_banner.clone(),
                status: UserStatus::Connected, // <-- Set status here
            };
            // Get posts for this thread
            let mut post_stmt = conn.prepare("SELECT id, author_id, content FROM posts WHERE thread_id = ?1").map_err(|e| e.to_string())?;
            let post_rows = post_stmt.query_map(params![thread_id.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            }).map_err(|e| e.to_string())?;
            let mut posts = Vec::new();
            for post_row in post_rows {
                let (post_id, post_author_id, content) = post_row.map_err(|e| e.to_string())?;
                // Get post author
                let mut post_user_stmt = conn.prepare("SELECT id, username, color, role FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
                let post_user_row = post_user_stmt.query_row(params![post_author_id.clone()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
                }).map_err(|e| e.to_string())?;
                let (puser_id, pusername, pcolor, prole) = post_user_row;
                let post_author_profile = futures::executor::block_on(db_get_user_profile(Uuid::parse_str(&puser_id).unwrap())).map_err(|e| e.to_string())?;
                let post_author = User {
                    id: Uuid::parse_str(&puser_id).unwrap(),
                    username: pusername,
                    color: parse_color(&pcolor),
                    role: match prole.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
                    profile_pic: post_author_profile.profile_pic.clone(),
                    cover_banner: post_author_profile.cover_banner.clone(),
                    status: UserStatus::Connected, // <-- Set status here
                };
                posts.push(Post {
                    id: Uuid::parse_str(&post_id).unwrap(),
                    author: post_author,
                    content,
                });
            }
            threads.push(Thread {
                id: thread_uuid,
                title,
                author,
                posts,
            });
        }
        forums.push(Forum {
            id: forum_uuid,
            name,
            description,
            threads,
        });
    }
    Ok(forums)
}

async fn db_create_thread(forum_id: Uuid, title: &str, author_id: Uuid, content: &str) -> Result<(), String> {
    let forum_id = forum_id.to_string();
    let title = title.to_string();
    let author_id = author_id.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let thread_id = Uuid::new_v4().to_string();
        let post_id = Uuid::new_v4().to_string();
        // Insert thread
        conn.execute(
            "INSERT INTO threads (id, forum_id, title, author_id) VALUES (?1, ?2, ?3, ?4)",
            params![thread_id, forum_id, title, author_id],
        ).map_err(|e| e.to_string())?;
        // Insert first post
        conn.execute(
            "INSERT INTO posts (id, thread_id, author_id, content) VALUES (?1, ?2, ?3, ?4)",
            params![post_id, thread_id, author_id, content],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

async fn db_create_post(thread_id: Uuid, author_id: Uuid, content: &str) -> Result<(), String> {
    let thread_id = thread_id.to_string();
    let author_id = author_id.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let post_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO posts (id, thread_id, author_id, content) VALUES (?1, ?2, ?3, ?4)",
            params![post_id, thread_id, author_id, content],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

// --- SQLite Server/Channel/Message Management ---

use common::{Server, Channel, ChannelPermissions, ChannelMessage};

async fn db_create_server(
    name: &str,
    description: &str,
    public: bool,
    owner: Uuid,
    icon: Option<&str>,
    banner: Option<&str>,
) -> Result<Uuid, String> {
    let name = name.to_string();
    let description = description.to_string();
    let icon = icon.map(|s| s.to_string());
    let banner = banner.map(|s| s.to_string());
    let owner = owner.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO servers (id, name, description, public, owner, icon, banner) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id.to_string(), name, description, public as i32, owner, icon, banner],
        ).map_err(|e| e.to_string())?;
        // Add owner to server_users and server_mods
        conn.execute(
            "INSERT INTO server_users (server_id, user_id) VALUES (?1, ?2)",
            params![id.to_string(), owner],
        ).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO server_mods (server_id, user_id) VALUES (?1, ?2)",
            params![id.to_string(), owner],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }).await.unwrap()
}

async fn db_create_channel(
    server_id: Uuid,
    name: &str,
    description: &str,
) -> Result<Uuid, String> {
    let server_id_str = server_id.to_string();
    let name = name.to_string();
    let description = description.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO channels (id, server_id, name, description) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), server_id_str, name, description],
        ).map_err(|e| e.to_string())?;
        // Add all server members to channel_users
        let mut stmt = conn.prepare("SELECT user_id FROM server_users WHERE server_id = ?1").map_err(|e| e.to_string())?;
        let user_rows = stmt.query_map(params![server_id_str.clone()], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
        for user_row in user_rows {
            let user_id = user_row.map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR IGNORE INTO channel_users (channel_id, user_id) VALUES (?1, ?2)",
                params![id.to_string(), user_id],
            ).ok();
        }
        Ok(id)
    }).await.unwrap()
}

async fn db_create_channel_message(
    channel_id: Uuid,
    sent_by: Uuid,
    timestamp: i64,
    content: &str,
) -> Result<Uuid, String> {
    let channel_id = channel_id.to_string();
    let sent_by = sent_by.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO channel_messages (id, channel_id, sent_by, timestamp, content) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), channel_id, sent_by, timestamp, content],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }).await.unwrap()
}

fn parse_color(color_str: &str) -> Color {
    match color_str {
        "Reset" => Color::Reset,
        "Black" => Color::Black,
        "Red" => Color::Red,
        "Green" => Color::Green,
        "Yellow" => Color::Yellow,
        "Blue" => Color::Blue,
        "Magenta" => Color::Magenta,
        "Cyan" => Color::Cyan,
        "Gray" => Color::Gray,
        "DarkGray" => Color::DarkGray,
        "LightRed" => Color::LightRed,
        "LightGreen" => Color::LightGreen,
        "LightYellow" => Color::LightYellow,
        "LightBlue" => Color::LightBlue,
        "LightMagenta" => Color::LightMagenta,
        "LightCyan" => Color::LightCyan,
        "White" => Color::White,
        _ => Color::Reset,
    }
}

// --- Mention extraction helper ---
fn extract_mentions(content: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let re = regex::Regex::new(r"@([a-zA-Z0-9_]+)").unwrap();
    for cap in re.captures_iter(content) {
        if let Some(username) = cap.get(1) {
            mentions.push(username.as_str().to_string());
        }
    }
    mentions
}

// --- Store direct message in DB ---
async fn db_store_direct_message(from_user_id: Uuid, to_user_id: Uuid, content: &str, timestamp: i64) -> Result<(), String> {
    let from_user_id = from_user_id.to_string();
    let to_user_id = to_user_id.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO direct_messages (id, from_user_id, to_user_id, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, from_user_id, to_user_id, content, timestamp],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

// Fetch all servers a user is a member of
async fn db_get_user_servers(user_id: Uuid) -> Result<Vec<Server>, String> {
    use common::{Server, Channel};
    let user_id = user_id.to_string();
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT s.id, s.name, s.description, s.public, s.invite_code, s.icon, s.banner, s.owner FROM servers s INNER JOIN server_users su ON s.id = su.server_id WHERE su.user_id = ?1").map_err(|e| e.to_string())?;
        let server_rows = stmt.query_map(params![user_id.clone()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i32>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, String>(7)?))
        }).map_err(|e| e.to_string())?;
        let mut servers = Vec::new();
        for server_row in server_rows {
            let (id, name, description, public, invite_code, icon, banner, owner) = server_row.map_err(|e| e.to_string())?;
            let server_id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
            // Fetch mods
            let mut mods_stmt = conn.prepare("SELECT user_id FROM server_mods WHERE server_id = ?1").map_err(|e| e.to_string())?;
            let mods = mods_stmt.query_map(params![id.clone()], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?
                .map(|r| Uuid::parse_str(&r.unwrap()).unwrap()).collect();
            // Fetch userlist
            let mut users_stmt = conn.prepare("SELECT user_id FROM server_users WHERE server_id = ?1").map_err(|e| e.to_string())?;
            let userlist = users_stmt.query_map(params![id.clone()], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?
                .map(|r| Uuid::parse_str(&r.unwrap()).unwrap()).collect();
            // Fetch only channels the user is a member of
            let mut channels_stmt = conn.prepare("SELECT id, name, description FROM channels WHERE server_id = ?1 AND id IN (SELECT channel_id FROM channel_users WHERE user_id = ?2)").map_err(|e| e.to_string())?;
            let channel_rows = channels_stmt.query_map(params![id.clone(), user_id.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            }).map_err(|e| e.to_string())?;
            let mut channels = Vec::new();
            for channel_row in channel_rows {
                let (chan_id, chan_name, chan_desc) = channel_row.map_err(|e| e.to_string())?;
                let channel_id = Uuid::parse_str(&chan_id).map_err(|e| e.to_string())?;
                // Fetch channel userlist
                let mut cu_stmt = conn.prepare("SELECT user_id FROM channel_users WHERE channel_id = ?1").map_err(|e| e.to_string())?;
                let channel_userlist = cu_stmt.query_map(params![chan_id.clone()], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?
                    .map(|r| Uuid::parse_str(&r.unwrap()).unwrap()).collect();
                // Fetch permissions
                let mut perm_stmt = conn.prepare("SELECT user_id, can_read, can_write FROM channel_permissions WHERE channel_id = ?1").map_err(|e| e.to_string())?;
                let mut can_read = Vec::new();
                let mut can_write = Vec::new();
                let perm_rows = perm_stmt.query_map(params![chan_id.clone()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?, row.get::<_, i32>(2)?))
                }).map_err(|e| e.to_string())?;
                for perm_row in perm_rows {
                    let (uid, read, write) = perm_row.map_err(|e| e.to_string())?;
                    let uuid = Uuid::parse_str(&uid).map_err(|e| e.to_string())?;
                    if read != 0 { can_read.push(uuid); }
                    if write != 0 { can_write.push(uuid); }
                }
                // Fetch messages (last 50)
                let mut msg_stmt = conn.prepare("SELECT id, sent_by, timestamp, content FROM channel_messages WHERE channel_id = ?1 ORDER BY timestamp ASC LIMIT 50").map_err(|e| e.to_string())?;
                let msg_rows = msg_stmt.query_map(params![chan_id.clone()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?))
                }).map_err(|e| e.to_string())?;
                let mut messages = Vec::new();
                for msg_row in msg_rows {
                    let (msg_id, sent_by, timestamp, content) = msg_row.map_err(|e| e.to_string())?;
                    let sent_by_uuid = Uuid::parse_str(&sent_by).map_err(|e| e.to_string())?;
                    let author_profile = futures::executor::block_on(db_get_user_by_id(sent_by_uuid)).map_err(|e| e.to_string())?;
                    messages.push(common::ChannelMessage {
                        id: Uuid::parse_str(&msg_id).map_err(|e| e.to_string())?,
                        channel_id,
                        sent_by: sent_by_uuid,
                        timestamp,
                        content,
                        author_username: author_profile.username,
                        author_color: author_profile.color,
                        author_profile_pic: author_profile.profile_pic,
                    });
                }
                channels.push(Channel {
                    id: channel_id,
                    server_id: server_id,
                    name: chan_name,
                    description: chan_desc,
                    permissions: common::ChannelPermissions { can_read, can_write },
                    userlist: channel_userlist,
                    messages,
                });
            }
            servers.push(Server {
                id: server_id,
                name,
                description,
                public: public != 0,
                invite_code,
                icon,
                banner,
                owner: Uuid::parse_str(&owner).map_err(|e| e.to_string())?,
                mods,
                userlist,
                channels,
            });
        }
        Ok(servers)
    }).await.unwrap()
}

async fn ensure_default_server_exists() -> Result<(), String> {
    use common::UserRole;
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM servers", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if count > 0 {
        return Ok(());
    }
    // Get the first admin user as owner
    let mut stmt = conn.prepare("SELECT id FROM users WHERE role = 'Admin' LIMIT 1").map_err(|e| e.to_string())?;
    let owner_id: String = stmt.query_row([], |row| row.get(0)).map_err(|_| "No admin user found".to_string())?;
    let owner_uuid = Uuid::parse_str(&owner_id).map_err(|e| e.to_string())?;
    // drop(conn);
    // Create server
    let server_id = db_create_server(
        "Nexus",
        "The default community server.",
        true,
        owner_uuid,
        None,
        None,
    ).await?;
    // Create default channels
    let _ = db_create_channel(server_id, "general", "General discussion").await?;
    let _ = db_create_channel(server_id, "cyberdeck", "Tech talk").await?;
    let _ = db_create_channel(server_id, "random", "Off-topic").await?;
    Ok(())
}