// server/src/main.rs

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use common::{
    create_initial_forums, ChatMessage, ClientMessage, Forum, Post, ServerMessage, Thread, User,
    UserProfile, UserRole,
};
use futures::{SinkExt, StreamExt};
use ratatui::style::Color;
use serde::{de::DeserializeOwned, Serialize};
use std::{collections::HashMap, env, error::Error, fs, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};
use uuid::Uuid;

const FORUM_DATA_PATH: &str = "forums.json";
const USER_DATA_PATH: &str = "users.json";

struct Peer {
    user_id: Option<Uuid>,
    tx: mpsc::UnboundedSender<ServerMessage>,
}

type PeerMap = Arc<Mutex<HashMap<Uuid, Peer>>>;
type ForumState = Arc<Mutex<Vec<Forum>>>;
type UserState = Arc<Mutex<Vec<UserProfile>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let listener = TcpListener::bind(&addr).await?;
    info!("Server listening on: {}", addr);

    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));
    let forums = load_or_create(FORUM_DATA_PATH, create_initial_forums)?;
    let users = load_or_create(USER_DATA_PATH, Vec::new)?;

    let forum_state = Arc::new(Mutex::new(forums));
    let user_state = Arc::new(Mutex::new(users));

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle_connection(
            stream,
            peer_map.clone(),
            forum_state.clone(),
            user_state.clone(),
        ));
    }
}

fn load_or_create<T: DeserializeOwned + Serialize>(
    path: &str,
    default: impl Fn() -> T,
) -> Result<T, Box<dyn Error>> {
    match fs::read_to_string(path) {
        Ok(data) => Ok(serde_json::from_str(&data)?),
        Err(_) => {
            warn!("{} not found, creating with default data.", path);
            let default_data = default();
            fs::write(path, serde_json::to_string_pretty(&default_data)?)?;
            Ok(default_data)
        }
    }
}

async fn save_data<T: Serialize>(path: &str, data: &Mutex<T>) -> Result<(), Box<dyn Error>> {
    let data_lock = data.lock().await;
    fs::write(path, serde_json::to_string_pretty(&*data_lock)?)?;
    Ok(())
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
    forum_state: ForumState,
    user_state: UserState,
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

        // *** FIX 1: Borrow `current_user` instead of moving it. ***
        if let Some(ref user) = current_user {
            match msg {
                ClientMessage::Logout => {
                    current_user = None;
                    peer_map.lock().await.get_mut(&conn_id).unwrap().user_id = None;
                }
                ClientMessage::UpdatePassword(new_password) => {
                    let mut users = user_state.lock().await;
                    if let Some(profile) = users.iter_mut().find(|u| u.id == user.id) {
                        profile.hash = hash_password(&new_password).unwrap();
                        save_data(USER_DATA_PATH, &user_state).await.unwrap();
                        // *** FIX 2: Hold the lock guard in a `let` binding. ***
                        let peers = peer_map.lock().await;
                        let tx = &peers.get(&conn_id).unwrap().tx;
                        tx.send(ServerMessage::Notification(
                            "Password updated.".into(),
                            false,
                        ))
                        .unwrap();
                    }
                }
                ClientMessage::UpdateColor(serializable_color) => {
                    let mut users = user_state.lock().await;
                    if let Some(profile) = users.iter_mut().find(|u| u.id == user.id) {
                        profile.color = serializable_color.0;
                        save_data(USER_DATA_PATH, &user_state).await.unwrap();
                        // *** FIX 2: Hold the lock guard in a `let` binding. ***
                        let peers = peer_map.lock().await;
                        let tx = &peers.get(&conn_id).unwrap().tx;
                        tx.send(ServerMessage::Notification("Color updated.".into(), false))
                            .unwrap();
                    }
                }
                ClientMessage::GetForums => {
                    let forums = forum_state.lock().await;
                    // *** FIX 2: Hold the lock guard in a `let` binding. ***
                    let peers = peer_map.lock().await;
                    let tx = &peers.get(&conn_id).unwrap().tx;
                    tx.send(ServerMessage::Forums(forums.clone())).unwrap();
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
                    let mut forums = forum_state.lock().await;
                    if let Some(forum) = forums.iter_mut().find(|f| f.id == forum_id) {
                        let new_post = Post {
                            id: Uuid::new_v4(),
                            author: user.clone(),
                            content,
                        };
                        let new_thread = Thread {
                            id: Uuid::new_v4(),
                            title,
                            author: user.clone(),
                            posts: vec![new_post],
                        };
                        forum.threads.push(new_thread);
                        save_data(FORUM_DATA_PATH, &forum_state).await.unwrap();
                        broadcast(&peer_map, &ServerMessage::Forums(forums.clone())).await;
                    }
                }
                ClientMessage::CreatePost { thread_id, content } => {
                    let mut forums = forum_state.lock().await;
                    if let Some(thread) = forums
                        .iter_mut()
                        .flat_map(|f| &mut f.threads)
                        .find(|t| t.id == thread_id)
                    {
                        let new_post = Post {
                            id: Uuid::new_v4(),
                            author: user.clone(),
                            content,
                        };
                        thread.posts.push(new_post);
                        save_data(FORUM_DATA_PATH, &forum_state).await.unwrap();
                        broadcast(&peer_map, &ServerMessage::Forums(forums.clone())).await;
                    }
                }
                _ => {} // Ignore other messages when logged in
            }
        } else { // Not logged in
            match msg {
                ClientMessage::Register { username, password } => {
                    {
                        let mut users = user_state.lock().await;
                        println!("[DEBUG] Users before registration: {:?}", users);
                        // *** FIX 2: Hold the lock guard in a `let` binding. ***
                        if users.iter().any(|u| u.username == username) {
                            let peers = peer_map.lock().await;
                            let tx = &peers.get(&conn_id).unwrap().tx;
                            tx.send(ServerMessage::AuthFailure("Username taken.".into())).unwrap();
                            continue;
                        } else {
                            let new_user_profile = UserProfile {
                                id: Uuid::new_v4(),
                                username: username.clone(),
                                hash: hash_password(&password).unwrap(),
                                color: Color::Green,
                                role: if users.is_empty() {
                                    UserRole::Admin
                                } else {
                                    UserRole::User
                                },
                            };
                            users.push(new_user_profile.clone());
                            println!("[DEBUG] Users after registration: {:?}", users);
                        }
                    } // lock released here
                    println!("[DEBUG] Attempting to save to {}", USER_DATA_PATH);
                    if let Err(e) = save_data(USER_DATA_PATH, &user_state).await {
                        println!("[ERROR] Failed to save users: {}", e);
                    } else {
                        println!("[DEBUG] Successfully saved users to {}", USER_DATA_PATH);
                    }
                    // Send AuthSuccess after saving
                    let users = user_state.lock().await;
                    let new_user_profile = users.iter().find(|u| u.username == username).unwrap().clone();
                    let user_data = User {
                        id: new_user_profile.id,
                        username: new_user_profile.username,
                        color: new_user_profile.color,
                        role: new_user_profile.role,
                    };
                    current_user = Some(user_data.clone());
                    peer_map.lock().await.get_mut(&conn_id).unwrap().user_id = Some(user_data.id);
                    let peers = peer_map.lock().await;
                    let tx = &peers.get(&conn_id).unwrap().tx;
                    tx.send(ServerMessage::AuthSuccess(user_data)).unwrap();
                }
                ClientMessage::Login { username, password } => {
                    // Lock user_state, get profile, drop lock
                    let profile = {
                        let users = user_state.lock().await;
                        users.iter().find(|u| u.username == username).cloned()
                    };

                    if let Some(profile) = profile {
                        println!("[DEBUG] Attempting login for username: {}", username);
                        println!("[DEBUG] Provided password: {}", password);
                        println!("[DEBUG] Stored hash: {}", profile.hash);
                        let verify = verify_password(&profile.hash, &password);
                        println!("[DEBUG] verify_password result: {}", verify);
                        if verify {
                            let user_data = User {
                                id: profile.id,
                                username: profile.username.clone(),
                                color: profile.color,
                                role: profile.role.clone(),
                            };
                            current_user = Some(user_data.clone());

                            // Lock peer_map, update peer, get tx, drop lock
                            let tx = {
                                let mut peers = peer_map.lock().await;
                                if let Some(peer) = peers.get_mut(&conn_id) {
                                    peer.user_id = Some(user_data.id);
                                    println!("[DEBUG] Updated peer_map for user: {}", user_data.username);
                                    peer.tx.clone()
                                } else {
                                    println!("[ERROR] Failed to find peer for conn_id: {}", conn_id);
                                    continue;
                                }
                            };

                            println!("[DEBUG] About to send AuthSuccess for user: {}", user_data.username);
                            match tx.send(ServerMessage::AuthSuccess(user_data.clone())) {
                                Ok(_) => println!("[DEBUG] AuthSuccess sent for user: {}", user_data.username),
                                Err(e) => println!("[ERROR] Failed to send AuthSuccess: {}", e),
                            }
                        } else {
                            let tx = {
                                let peers = peer_map.lock().await;
                                peers.get(&conn_id).unwrap().tx.clone()
                            };
                            tx.send(ServerMessage::AuthFailure(
                                "Invalid credentials.".into(),
                            ))
                            .unwrap();
                        }
                    } else {
                        println!("[DEBUG] Username not found: {}", username);
                        let tx = {
                            let peers = peer_map.lock().await;
                            peers.get(&conn_id).unwrap().tx.clone()
                        };
                        tx.send(ServerMessage::AuthFailure(
                            "Invalid credentials.".into(),
                        ))
                        .unwrap();
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