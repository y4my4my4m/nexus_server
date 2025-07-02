use crate::api::connection::PeerMap;
use nexus_tui_common::{ServerMessage, User};
use std::collections::HashSet;
use tracing::{error, info};
use uuid::Uuid;

pub struct BroadcastService;

impl BroadcastService {
    /// Broadcast a message to all authenticated users
    pub async fn broadcast_to_all(peer_map: &PeerMap, message: &ServerMessage) {
        let peers = peer_map.lock().await;
        let mut success_count = 0;
        let mut error_count = 0;

        for peer in peers.values() {
            if peer.user_id.is_some() {
                match peer.tx.send(message.clone()) {
                    Ok(_) => success_count += 1,
                    Err(e) => {
                        error_count += 1;
                        error!("Failed to broadcast message: {}", e);
                    }
                }
            }
        }

        info!(
            "Broadcasted message to {} users ({} errors)",
            success_count, error_count
        );
    }

    /// Broadcast a message to specific users
    pub async fn broadcast_to_users(
        peer_map: &PeerMap,
        user_ids: &[Uuid],
        message: &ServerMessage,
    ) {
        let peers = peer_map.lock().await;
        let user_ids_set: HashSet<Uuid> = user_ids.iter().copied().collect();
        let mut success_count = 0;

        for peer in peers.values() {
            if let Some(uid) = peer.user_id {
                if user_ids_set.contains(&uid) {
                    match peer.tx.send(message.clone()) {
                        Ok(_) => success_count += 1,
                        Err(e) => error!("Failed to send message to user {}: {}", uid, e),
                    }
                }
            }
        }

        info!("Sent message to {} users", success_count);
    }

    /// Send a message to multiple users
    async fn send_to_users(peer_map: &PeerMap, user_ids: &[Uuid], message: ServerMessage) {
        let peers = peer_map.lock().await;
        let user_ids_set: HashSet<Uuid> = user_ids.iter().copied().collect();
        let mut success_count = 0;

        for peer in peers.values() {
            if let Some(uid) = peer.user_id {
                if user_ids_set.contains(&uid) {
                    match peer.tx.send(message.clone()) {
                        Ok(_) => success_count += 1,
                        Err(e) => error!("Failed to send message to user {}: {}", uid, e),
                    }
                }
            }
        }

        info!("Sent message to {} users", success_count);
    }

    /// Broadcast user status change to users who share channels
    pub async fn broadcast_user_status_change(peer_map: &PeerMap, user: &User, joined: bool) {
        // Get users who share channels with this user
        let shared_users = match crate::db::channels::db_get_users_sharing_channels_with(user.id).await {
            Ok(users) => users,
            Err(e) => {
                error!("Failed to get shared channel users: {}", e);
                return;
            }
        };

        let message = if joined {
            ServerMessage::UserJoined(user.clone())
        } else {
            ServerMessage::UserLeft(user.id)
        };

        Self::send_to_users(peer_map, &shared_users, message).await;
    }

    /// Broadcast user profile update to users who share channels
    pub async fn broadcast_user_update(peer_map: &PeerMap, updated_user: &User) {
        // Get users who share channels with this user
        let shared_users = match crate::db::channels::db_get_users_sharing_channels_with(updated_user.id).await {
            Ok(users) => users,
            Err(e) => {
                error!("Failed to get shared channel users: {}", e);
                return;
            }
        };

        let message = ServerMessage::UserUpdated(updated_user.clone());
        Self::send_to_users(peer_map, &shared_users, message).await;
    }

    /// Broadcast to users in specific channels
    pub async fn broadcast_to_channel_users(
        peer_map: &PeerMap,
        channel_user_ids: &[Uuid],
        message: &ServerMessage,
    ) {
        Self::broadcast_to_users(peer_map, channel_user_ids, message).await;
    }

    /// Send a direct message to a specific user if they're online
    pub async fn send_to_user(peer_map: &PeerMap, user_id: Uuid, message: &ServerMessage) -> bool {
        let peers = peer_map.lock().await;
        
        for peer in peers.values() {
            if peer.user_id == Some(user_id) {
                match peer.tx.send(message.clone()) {
                    Ok(_) => return true,
                    Err(e) => {
                        error!("Failed to send message to user {}: {}", user_id, e);
                        return false;
                    }
                }
            }
        }
        
        false // User not online
    }

    /// Get list of online user IDs
    pub async fn get_online_users(peer_map: &PeerMap) -> HashSet<Uuid> {
        let peers = peer_map.lock().await;
        peers
            .values()
            .filter_map(|peer| peer.user_id)
            .collect()
    }

    /// Check if a user is online
    pub async fn is_user_online(peer_map: &PeerMap, user_id: Uuid) -> bool {
        let peers = peer_map.lock().await;
        peers.values().any(|peer| peer.user_id == Some(user_id))
    }
}