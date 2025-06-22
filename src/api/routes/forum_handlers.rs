use super::MessageRouter;
use crate::db;
use common::{ServerMessage, User};
use tokio::sync::mpsc;
use uuid::Uuid;

impl MessageRouter {
    /// Handle get forums
    pub async fn handle_get_forums(
        &self,
        response_sender: &mpsc::UnboundedSender<ServerMessage>,
    ) -> crate::errors::Result<()> {
        let forums = db::forums::db_get_forums().await.unwrap_or_default();
        self.send_response(response_sender, ServerMessage::Forums(forums));
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
                    
                    // Refresh forums to show new thread
                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::Forums(forums));
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
            match db::forums::db_create_post(thread_id, user.id, &content).await {
                Ok(_) => {
                    self.send_success(response_sender, "Post created successfully");
                    
                    // Refresh forums to show new post
                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::Forums(forums));
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
                    
                    // Refresh forums to show updated state
                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::Forums(forums));
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
                    
                    // Refresh forums to show updated state
                    let forums = db::forums::db_get_forums().await.unwrap_or_default();
                    self.send_response(response_sender, ServerMessage::Forums(forums));
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