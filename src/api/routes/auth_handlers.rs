use super::MessageRouter;
use crate::services::UserService;
use common::{ServerMessage, User, UserColor};
use tokio::sync::mpsc;
use uuid::Uuid;

impl MessageRouter {
    /// Handle user registration
    pub async fn handle_register(
        &self,
        username: String,
        password: String,
        current_user: &mut Option<User>,
        peer_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        match UserService::register(&username, &password, &self.peer_map).await {
            Ok(user) => {
                // Update peer map
                let mut peers = self.peer_map.lock().await;
                if let Some(peer) = peers.get_mut(&peer_id) {
                    peer.user_id = Some(user.id);
                }
                drop(peers);
                
                *current_user = Some(user.clone());
                self.send_response(response_sender, ServerMessage::AuthSuccess(user));
            }
            Err(e) => {
                self.send_response(response_sender, ServerMessage::AuthFailure(e.to_string()));
            }
        }
        Ok(())
    }

    /// Handle user login
    pub async fn handle_login(
        &self,
        username: String,
        password: String,
        current_user: &mut Option<User>,
        peer_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        match UserService::login(&username, &password, &self.peer_map).await {
            Ok(user) => {
                // Update peer map
                let mut peers = self.peer_map.lock().await;
                if let Some(peer) = peers.get_mut(&peer_id) {
                    peer.user_id = Some(user.id);
                }
                drop(peers);
                
                *current_user = Some(user.clone());
                self.send_response(response_sender, ServerMessage::AuthSuccess(user));
            }
            Err(e) => {
                self.send_response(response_sender, ServerMessage::AuthFailure(e.to_string()));
            }
        }
        Ok(())
    }

    /// Handle user logout
    pub async fn handle_logout(
        &self,
        current_user: &mut Option<User>,
        peer_id: Uuid,
        _response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            UserService::logout(user, &self.peer_map).await;
        }
        
        // Clear peer map
        let mut peers = self.peer_map.lock().await;
        if let Some(peer) = peers.get_mut(&peer_id) {
            peer.user_id = None;
        }
        drop(peers);
        
        *current_user = None;
        Ok(())
    }

    /// Handle password update
    pub async fn handle_update_password(
        &self,
        current_user: &Option<User>,
        new_password: String,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match UserService::update_password(user.id, &new_password).await {
                Ok(_) => {
                    self.send_success(response_sender, "Password updated successfully!");
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to update password: {}", e));
                }
            }
        }
        Ok(())
    }

    /// Handle color update
    pub async fn handle_update_color(
        &self,
        current_user: &mut Option<User>,
        color: UserColor,
        _response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            let color_str = color.0;
            if let Ok(updated_user) = UserService::update_color(user.id, &color_str, &self.peer_map).await {
                *current_user = Some(updated_user);
            }
        }
        Ok(())
    }

    /// Handle profile update
    pub async fn handle_update_profile(
        &self,
        current_user: &Option<User>,
        bio: Option<String>,
        url1: Option<String>,
        url2: Option<String>,
        url3: Option<String>,
        location: Option<String>,
        profile_pic: Option<String>,
        cover_banner: Option<String>,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match UserService::update_profile(
                user.id, bio, url1, url2, url3, location, profile_pic, cover_banner, &self.peer_map
            ).await {
                Ok(_profile) => {
                    self.send_success(response_sender, "Profile updated successfully!");
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to update profile: {}", e));
                }
            }
        } else {
            self.send_error(response_sender, "Must be logged in to update profile");
        }
        Ok(())
    }

    /// Handle get profile
    pub async fn handle_get_profile(
        &self,
        user_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        match UserService::get_profile(user_id).await {
            Ok(profile) => {
                self.send_response(response_sender, ServerMessage::Profile(profile));
            }
            Err(e) => {
                self.send_error(response_sender, &format!("Failed to load profile: {}", e));
            }
        }
        Ok(())
    }

    /// Handle get user list
    pub async fn handle_get_user_list(
        &self,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        match UserService::get_user_list(&self.peer_map).await {
            Ok(users) => {
                self.send_response(response_sender, ServerMessage::UserList(users));
            }
            Err(_) => {
                self.send_error(response_sender, "Failed to get user list");
            }
        }
        Ok(())
    }
}