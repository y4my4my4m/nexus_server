// server/src/main.rs

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use common::{
    ChatMessage, ClientMessage, Forum, Post, ServerMessage, Thread, User,
    UserProfile, UserRole,
};
use futures::{SinkExt, StreamExt};
use ratatui::style::Color;
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{de::DeserializeOwned, Serialize};
use std::{collections::HashMap, env, error::Error, fs, path::Path, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
};
use tokio::task;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};
use uuid::Uuid;

const FORUM_DATA_PATH: &str = "forums.json";
const USER_DATA_PATH: &str = "users.json";
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

    // Migrate forums from JSON file to DB if needed
    migrate_forums_json_to_db().await?;
    migrate_users_json_to_db().await?;

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
                                    });
                                    let peers = peer_map.lock().await;
                                    let tx = &peers.get(&conn_id).unwrap().tx;
                                    // Notify client with updated user info
                                    tx.send(ServerMessage::AuthSuccess(current_user.as_ref().unwrap().clone())).unwrap();
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
                ClientMessage::SendChatMessage(content) => {
                    let chat_msg = ServerMessage::NewChatMessage(ChatMessage {
                        author: user.username.clone(),
                        content,
                        color: user.color,
                    });
                    broadcast(&peer_map, &chat_msg).await;
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
                            let user_data = User {
                                id: profile.id,
                                username: profile.username.clone(),
                                color: profile.color,
                                role: profile.role.clone(),
                            };
                            current_user = Some(user_data.clone());
                            peer_map.lock().await.get_mut(&conn_id).unwrap().user_id = Some(user_data.id);
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::AuthSuccess(user_data)).unwrap();
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
                            let user_data = User {
                                id: profile.id,
                                username: profile.username.clone(),
                                color: profile.color,
                                role: profile.role.clone(),
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
                            tx.send(ServerMessage::AuthSuccess(user_data)).unwrap();
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
    Ok(conn)
}

// --- SQLite User Management ---

async fn db_get_user_by_id(user_id: Uuid) -> Result<UserProfile, String> {
    let user_id = user_id.to_string();
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT id, username, password_hash, color, role FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
        let user = stmt.query_row(params![user_id], |row| {
            let id: String = row.get(0)?;
            let username: String = row.get(1)?;
            let hash: String = row.get(2)?;
            let color: String = row.get(3)?;
            let role: String = row.get(4)?;
            Ok((id, username, hash, color, role))
        }).map_err(|_| "User not found".to_string())?;
        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: user.2,
            color: parse_color(&user.3),
            role: match user.4.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
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
            hash: user.2,
            color: parse_color(&user.3),
            role: match user.4.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
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

// --- SQLite Forum/Thread/Post Management ---

async fn db_get_forums() -> Result<Vec<Forum>, String> {
    tokio::task::spawn_blocking(move || {
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
            let thread_rows = thread_stmt.query_map(params![forum_id], |row| {
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
                let author = User {
                    id: Uuid::parse_str(&user_id).unwrap(),
                    username,
                    color: parse_color(&color),
                    role: match role.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
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
                    let post_author = User {
                        id: Uuid::parse_str(&puser_id).unwrap(),
                        username: pusername,
                        color: parse_color(&pcolor),
                        role: match prole.as_str() {
                            "Admin" => UserRole::Admin,
                            "Moderator" => UserRole::Moderator,
                            _ => UserRole::User,
                        },
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
    }).await.unwrap()
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

async fn migrate_forums_json_to_db() -> Result<(), Box<dyn Error>> {
    if !Path::new(FORUM_DATA_PATH).exists() {
        return Ok(());
    }
    let conn = Connection::open(DB_PATH)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM forums", [], |row| row.get(0))?;
    if count > 0 {
        return Ok(()); // Already migrated
    }
    let data = std::fs::read_to_string(FORUM_DATA_PATH)?;
    let forums: Vec<Forum> = serde_json::from_str(&data)?;
    use std::collections::HashSet;
    let mut user_set = HashSet::new();
    // Collect all unique users from threads and posts
    for forum in &forums {
        for thread in &forum.threads {
            user_set.insert((&thread.author.id, &thread.author.username, &thread.author.color, &thread.author.role));
            for post in &thread.posts {
                user_set.insert((&post.author.id, &post.author.username, &post.author.color, &post.author.role));
            }
        }
    }
    // Insert all users first
    for (id, username, color, role) in user_set {
        conn.execute(
            "INSERT OR IGNORE INTO users (id, username, password_hash, color, role) VALUES (?1, ?2, '', ?3, ?4)",
            params![id.to_string(), username, format!("{:?}", color), format!("{:?}", role)],
        )?;
    }
    // Now insert forums, threads, posts
    for forum in forums {
        conn.execute(
            "INSERT INTO forums (id, name, description) VALUES (?1, ?2, ?3)",
            params![forum.id.to_string(), forum.name, forum.description],
        )?;
        for thread in forum.threads {
            conn.execute(
                "INSERT INTO threads (id, forum_id, title, author_id) VALUES (?1, ?2, ?3, ?4)",
                params![thread.id.to_string(), forum.id.to_string(), thread.title, thread.author.id.to_string()],
            )?;
            for post in thread.posts {
                conn.execute(
                    "INSERT INTO posts (id, thread_id, author_id, content) VALUES (?1, ?2, ?3, ?4)",
                    params![post.id.to_string(), thread.id.to_string(), post.author.id.to_string(), post.content],
                )?;
            }
        }
    }
    Ok(())
}

async fn migrate_users_json_to_db() -> Result<(), Box<dyn Error>> {
    if !Path::new(USER_DATA_PATH).exists() {
        return Ok(());
    }
    let conn = Connection::open(DB_PATH)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
    if count > 0 {
        return Ok(()); // Already migrated
    }
    let data = std::fs::read_to_string(USER_DATA_PATH)?;
    let users: Vec<serde_json::Value> = serde_json::from_str(&data)?;
    for user in users {
        let id = user["id"].as_str().unwrap();
        let username = user["username"].as_str().unwrap();
        let hash = user["password_hash"].as_str().unwrap();
        let color = user["color"].as_str().unwrap();
        let role = user["role"].as_str().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO users (id, username, password_hash, color, role) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, username, hash, color, role],
        )?;
    }
    Ok(())
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