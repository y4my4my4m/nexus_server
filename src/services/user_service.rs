use crate::db::users;
use crate::errors::{Result, ServerError};
use crate::services::BroadcastService;
use crate::api::connection::PeerMap;
use common::{User, UserProfile, UserStatus};
use tracing::{error, info};
use uuid::Uuid;

pub struct UserService;

impl UserService {
    /// Register a new user
    pub async fn register(
        username: &str,
        password: &str,
        peer_map: &PeerMap,
    ) -> Result<User> {
        // Check if this is the first user (make them admin)
        let is_first_user = users::db_count_users().await? == 0;
        let role = if is_first_user { "Admin" } else { "User" };
        
        // Register user in database
        let profile = users::db_register_user(username, password, "Green", role).await
            .map_err(|e| ServerError::Database(e))?;

        // Add user to default server and channels
        if let Err(e) = Self::add_user_to_default_server(profile.id).await {
            error!("Failed to add new user to default server: {}", e);
        }

        // Create User object with online status
        let user = User {
            id: profile.id,
            username: profile.username.clone(),
            color: profile.color.into(),
            role: profile.role,
            profile_pic: profile.profile_pic,
            cover_banner: profile.cover_banner,
            status: UserStatus::Connected,
        };

        // Broadcast user joined to relevant users
        BroadcastService::broadcast_user_status_change(peer_map, &user, true).await;
        
        info!("User registered: {}", user.username);
        Ok(user)
    }

    /// Login user
    pub async fn login(
        username: &str,
        password: &str,
        peer_map: &PeerMap,
    ) -> Result<User> {
        let profile = users::db_login_user(username, password).await
            .map_err(|e| ServerError::Authentication(e))?;

        let user = User {
            id: profile.id,
            username: profile.username.clone(),
            color: profile.color.into(),
            role: profile.role,
            profile_pic: profile.profile_pic,
            cover_banner: profile.cover_banner,
            status: UserStatus::Connected,
        };

        // Broadcast user joined
        BroadcastService::broadcast_user_status_change(peer_map, &user, true).await;
        
        info!("User logged in: {}", user.username);
        Ok(user)
    }

    /// Logout user
    pub async fn logout(user: &User, peer_map: &PeerMap) {
        // Broadcast user left
        BroadcastService::broadcast_user_status_change(peer_map, user, false).await;
        
        info!("User logged out: {}", user.username);
    }

    /// Update user profile
    pub async fn update_profile(
        user_id: Uuid,
        bio: Option<String>,
        url1: Option<String>,
        url2: Option<String>,
        url3: Option<String>,
        location: Option<String>,
        profile_pic: Option<String>,
        cover_banner: Option<String>,
        peer_map: &PeerMap,
    ) -> Result<UserProfile> {
        // Update profile in database
        users::db_update_user_profile(
            user_id, bio, url1, url2, url3, location, profile_pic, cover_banner
        ).await.map_err(|e| ServerError::Database(e))?;

        // Get updated profile
        let profile = users::db_get_user_profile(user_id).await
            .map_err(|e| ServerError::Database(e))?;

        // Create updated user object and broadcast
        if let Ok(full_user) = users::db_get_user_by_id(user_id).await {
            let updated_user = User {
                id: full_user.id,
                username: full_user.username,
                color: full_user.color.into(),
                role: full_user.role,
                profile_pic: full_user.profile_pic,
                cover_banner: full_user.cover_banner,
                status: UserStatus::Connected,
            };

            BroadcastService::broadcast_user_update(peer_map, &updated_user).await;
        }

        info!("Profile updated for user: {}", user_id);
        Ok(profile)
    }

    /// Update user color
    pub async fn update_color(
        user_id: Uuid,
        color: &str,
        peer_map: &PeerMap,
    ) -> Result<User> {
        // Update color in database
        users::db_update_user_color(user_id, color).await
            .map_err(|e| ServerError::Database(e))?;

        // Get updated user
        let profile = users::db_get_user_by_id(user_id).await
            .map_err(|e| ServerError::Database(e))?;

        let updated_user = User {
            id: profile.id,
            username: profile.username,
            color: profile.color.into(),
            role: profile.role,
            profile_pic: profile.profile_pic,
            cover_banner: profile.cover_banner,
            status: UserStatus::Connected,
        };

        // Broadcast user update
        BroadcastService::broadcast_user_update(peer_map, &updated_user).await;

        info!("Color updated for user: {}", updated_user.username);
        Ok(updated_user)
    }

    /// Update user password
    pub async fn update_password(user_id: Uuid, new_password: &str) -> Result<()> {
        users::db_update_user_password(user_id, new_password).await
            .map_err(|e| ServerError::Database(e))?;

        info!("Password updated for user: {}", user_id);
        Ok(())
    }

    /// Get user profile
    pub async fn get_profile(user_id: Uuid) -> Result<UserProfile> {
        users::db_get_user_profile(user_id).await
            .map_err(|e| ServerError::Database(e))
    }

    /// Get list of online users with updated status
    pub async fn get_user_list(peer_map: &PeerMap) -> Result<Vec<User>> {
        let online_users = BroadcastService::get_online_users(peer_map).await;
        let mut users = Vec::new();

        for user_id in online_users {
            if let Ok(profile) = users::db_get_user_by_id(user_id).await {
                users.push(User {
                    id: profile.id,
                    username: profile.username,
                    color: profile.color.into(),
                    role: profile.role,
                    profile_pic: profile.profile_pic,
                    cover_banner: profile.cover_banner,
                    status: UserStatus::Connected,
                });
            }
        }

        Ok(users)
    }

    /// Add user to default server (for new registrations)
    async fn add_user_to_default_server(user_id: Uuid) -> Result<()> {
        // Get default server ID
        let default_server_id = crate::db::servers::get_default_server_id().await
            .map_err(|e| ServerError::Database(e))?;

        if let Some(server_id) = default_server_id {
            // Add to server
            crate::db::servers::add_user_to_server(server_id, user_id).await
                .map_err(|e| ServerError::Database(e))?;

            // Add to all channels in that server
            let channel_ids = crate::db::channels::get_server_channels(server_id).await
                .map_err(|e| ServerError::Database(e))?;

            for channel_id in channel_ids {
                let _ = crate::db::channels::add_user_to_channel(channel_id, user_id).await;
            }
        }

        Ok(())
    }
}