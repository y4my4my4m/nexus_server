use super::MessageRouter;
use crate::db;
use common::{ServerMessage, User};
use tokio::sync::mpsc;
use uuid::Uuid;

impl MessageRouter {
    /// Handle get forums - use lightweight version by default for better performance
    pub async fn handle_get_forums(
        &self,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
        self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
        Ok(())
    }

    /// Handle create forum (Admin only)
    pub async fn handle_create_forum(
        &self,
        current_user: &Option<User>,
        name: String,
        description: String,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            if user.role == common::UserRole::Admin {
                match db::forums::db_create_forum(&name, &description).await {
                    Ok(_) => {
                        self.send_success(response_sender, "Forum created successfully");
                        
                        // Refresh forums to show new forum - use lightweight version
                        let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
                        self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
                    }
                    Err(e) => {
                        self.send_error(response_sender, &format!("Failed to create forum: {}", e));
                    }
                }
            } else {
                self.send_error(response_sender, "Only admins can create forums");
            }
        } else {
            self.send_error(response_sender, "Must be logged in to create forums");
        }
        Ok(())
    }

    /// Handle delete forum (Admin only)
    pub async fn handle_delete_forum(
        &self,
        current_user: &Option<User>,
        forum_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            if user.role == common::UserRole::Admin {
                match db::forums::db_delete_forum(forum_id).await {
                    Ok(_) => {
                        self.send_success(response_sender, "Forum deleted successfully");
                        
                        // Refresh forums to show updated list - use lightweight version
                        let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
                        self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
                    }
                    Err(e) => {
                        self.send_error(response_sender, &format!("Failed to delete forum: {}", e));
                    }
                }
            } else {
                self.send_error(response_sender, "Only admins can delete forums");
            }
        } else {
            self.send_error(response_sender, "Must be logged in to delete forums");
        }
        Ok(())
    }

    /// Handle create thread
    pub async fn handle_create_thread(
        &self,
        current_user: &Option<User>,
        forum_id: Uuid,
        title: String,
        content: String,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match db::forums::db_create_thread(forum_id, &title, user.id, &content).await {
                Ok(_) => {
                    self.send_success(response_sender, "Thread created successfully");
                    
                    // Refresh forums to show new thread - use lightweight version
                    let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to create thread: {}", e));
                }
            }
        } else {
            self.send_error(response_sender, "Must be logged in to create threads");
        }
        Ok(())
    }

    /// Handle create post
    pub async fn handle_create_post(
        &self,
        current_user: &Option<User>,
        thread_id: Uuid,
        content: String,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match db::forums::db_create_post(thread_id, user.id, &content, None).await {
                Ok(_) => {
                    self.send_success(response_sender, "Post created successfully");
                    
                    // Refresh forums to show new post - use lightweight version
                    let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to create post: {}", e));
                }
            }
        } else {
            self.send_error(response_sender, "Must be logged in to create posts");
        }
        Ok(())
    }

    /// Handle create post reply
    pub async fn handle_create_post_reply(
        &self,
        current_user: &Option<User>,
        thread_id: Uuid,
        content: String,
        reply_to: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match db::forums::db_create_post(thread_id, user.id, &content, Some(reply_to)).await {
                Ok(_) => {
                    // Don't send a success notification - it's annoying and useless
                    
                    // Create notification for the original post author if it's not a self-reply
                    if let Ok(original_post_author_id) = db::forums::db_get_post_author(reply_to).await {
                        if original_post_author_id != user.id {
                            // Create thread reply notification with the user's profile picture
                            crate::services::NotificationService::create_thread_reply_notification(
                                original_post_author_id,
                                thread_id,
                                &user.username,
                                user.profile_pic.as_deref(),
                                &self.peer_map,
                            ).await;
                        }
                    }
                    
                    // Refresh forums to show new reply - use lightweight version
                    let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to create reply: {}", e));
                }
            }
        } else {
            self.send_error(response_sender, "Must be logged in to create replies");
        }
        Ok(())
    }

    /// Handle delete post
    pub async fn handle_delete_post(
        &self,
        current_user: &Option<User>,
        post_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match db::forums::db_delete_post(post_id, user.id).await {
                Ok(_) => {
                    self.send_success(response_sender, "Post deleted successfully");
                    
                    // Refresh forums to show updated state - use lightweight version
                    let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to delete post: {}", e));
                }
            }
        } else {
            self.send_error(response_sender, "Must be logged in to delete posts");
        }
        Ok(())
    }

    /// Handle delete thread
    pub async fn handle_delete_thread(
        &self,
        current_user: &Option<User>,
        thread_id: Uuid,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        if let Some(user) = current_user {
            match db::forums::db_delete_thread(thread_id, user.id).await {
                Ok(_) => {
                    self.send_success(response_sender, "Thread deleted successfully");
                    
                    // Refresh forums to show updated state - use lightweight version
                    let forums = db::forums::db_get_forums_lightweight().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::ForumsLightweight(forums));
                }
                Err(e) => {
                    self.send_error(response_sender, &format!("Failed to delete thread: {}", e));
                }
            }
        } else {
            self.send_error(response_sender, "Must be logged in to delete threads");
        }
        Ok(())
    }
}