use common::{create_initial_forums, ChatMessage, ClientMessage, Forum, ServerMessage};
use futures::{SinkExt, StreamExt};
use ratatui::style::Color;
use std::{collections::HashMap, env, error::Error, fs, net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex}, // <-- Use Tokio's Mutex
};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};

const FORUM_DATA_PATH: &str = "forums.json";

type Tx = mpsc::UnboundedSender<ServerMessage>;
// The Arc/Mutex pattern now uses tokio::sync::Mutex
type PeerMap = Arc<Mutex<HashMap<SocketAddr, (Tx, String)>>>;
type ForumState = Arc<Mutex<Vec<Forum>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let listener = TcpListener::bind(&addr).await?;
    info!("Server listening on: {}", addr);

    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));
    let forums = load_or_create_forums()?;
    let forum_state = Arc::new(Mutex::new(forums));

    loop {
        let (stream, addr) = listener.accept().await?;
        info!("Accepted connection from: {}", addr);
        tokio::spawn(handle_connection(
            stream,
            addr,
            peer_map.clone(),
            forum_state.clone(),
        ));
    }
}

// This function doesn't need to be async, it's just doing sync file IO on startup
fn load_or_create_forums() -> Result<Vec<Forum>, Box<dyn Error>> {
    match fs::read_to_string(FORUM_DATA_PATH) {
        Ok(data) => {
            info!("Loaded forums from {}", FORUM_DATA_PATH);
            Ok(serde_json::from_str(&data)?)
        }
        Err(_) => {
            warn!(
                "{} not found, creating with initial data.",
                FORUM_DATA_PATH
            );
            let forums = create_initial_forums();
            fs::write(FORUM_DATA_PATH, serde_json::to_string_pretty(&forums)?)?;
            Ok(forums)
        }
    }
}

// save_forums now needs to be async to lock the tokio::sync::Mutex
async fn save_forums(forums: &ForumState) -> Result<(), Box<dyn Error>> {
    let forums_lock = forums.lock().await;
    fs::write(FORUM_DATA_PATH, serde_json::to_string_pretty(&*forums_lock)?)?;
    info!("Saved forum data to {}", FORUM_DATA_PATH);
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    peer_map: PeerMap,
    forum_state: ForumState,
) {
    // *** FIX 1: Split the Framed stream ***
    let framed = Framed::new(stream, LengthDelimitedCodec::new());
    let (mut sink, mut stream) = framed.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();

    // Add the peer to the map
    peer_map.lock().await.insert(addr, (tx, addr.to_string()));

    // This spawned task now owns the `sink` (write half)
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let bytes = bincode::serialize(&msg).unwrap();
            if let Err(e) = sink.send(bytes.into()).await {
                error!("Failed to send message to {}: {}", addr, e);
                break;
            }
        }
    });

    let mut current_username = addr.to_string();
    // This loop now owns the `stream` (read half)
    loop {
        match stream.next().await {
            Some(Ok(data)) => {
                let msg: ClientMessage = match bincode::deserialize(&data[..]) {
                    Ok(msg) => msg,
                    Err(e) => {
                        error!("Failed to deserialize message from {}: {}", addr, e);
                        continue;
                    }
                };

                // *** FIX 2: All locking is now async ***
                match msg {
                    ClientMessage::SetUsername(name) => {
                        info!("{} set username to {}", addr, name);
                        if let Some((_, username)) = peer_map.lock().await.get_mut(&addr) {
                            *username = name.clone();
                            current_username = name.clone();
                        }
                        let announcement = ServerMessage::NewChatMessage(ChatMessage {
                            author: "SYSTEM".to_string(),
                            content: format!("{} has connected.", name),
                            color: Color::Yellow,
                        });
                        broadcast(&peer_map, &announcement).await;
                    }
                    ClientMessage::GetForums => {
                        let forums_lock = forum_state.lock().await;
                        let msg = ServerMessage::Forums(forums_lock.clone());
                        if let Some((tx, _)) = peer_map.lock().await.get(&addr) {
                            if tx.send(msg).is_err() {
                                warn!("Failed to send forum list to {}", addr);
                            }
                        }
                    }
                    ClientMessage::SendChatMessage(content) => {
                        let chat_msg = ServerMessage::NewChatMessage(ChatMessage {
                            author: current_username.clone(),
                            content,
                            color: Color::Green,
                        });
                        broadcast(&peer_map, &chat_msg).await;
                    }
                    ClientMessage::AddThread { forum_idx, thread } => {
                        let mut forums = forum_state.lock().await;
                        if let Some(forum) = forums.get_mut(forum_idx) {
                            forum.threads.push(thread);
                            if let Err(e) = save_forums(&forum_state).await {
                                error!("Failed to save forums: {}", e);
                            }
                            let update_msg = ServerMessage::Forums(forums.clone());
                            broadcast(&peer_map, &update_msg).await;
                        }
                    }
                    ClientMessage::AddPost {
                        forum_idx,
                        thread_idx,
                        post,
                    } => {
                        let mut forums = forum_state.lock().await;
                        if let Some(forum) = forums.get_mut(forum_idx) {
                            if let Some(thread) = forum.threads.get_mut(thread_idx) {
                                thread.posts.push(post);
                                if let Err(e) = save_forums(&forum_state).await {
                                    error!("Failed to save forums: {}", e);
                                }
                                let update_msg = ServerMessage::Forums(forums.clone());
                                broadcast(&peer_map, &update_msg).await;
                            }
                        }
                    }
                }
            }
            Some(Err(e)) => {
                error!("Error reading from stream for {}: {}", addr, e);
                break;
            }
            None => {
                info!("Connection from {} closed", addr);
                break;
            }
        }
    }

    // Cleanup after disconnection
    peer_map.lock().await.remove(&addr);
    let announcement = ServerMessage::NewChatMessage(ChatMessage {
        author: "SYSTEM".to_string(),
        content: format!("{} has disconnected.", current_username),
        color: Color::Yellow,
    });
    broadcast(&peer_map, &announcement).await;
}

async fn broadcast(peer_map: &PeerMap, msg: &ServerMessage) {
    let peers = peer_map.lock().await;
    for (addr, (tx, _)) in peers.iter() {
        if tx.send(msg.clone()).is_err() {
            warn!("Failed to send message to peer {}, likely disconnected.", addr);
        }
    }
}