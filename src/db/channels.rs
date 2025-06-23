// Channel DB functions

use crate::db::db_config;
use crate::util::parse_user_color;
use common::{ChannelMessage, User, UserRole, UserStatus, UserInfo};
use rusqlite::{params, Connection};
use tokio::task;
use uuid::Uuid;

pub async fn db_create_channel(
    server_id: Uuid,
    name: &str,
    description: &str,
) -> Result<Uuid, String> {
    let server_id_str = server_id.to_string();
    let name = name.to_string();
    let description = description.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO channels (id, server_id, name, description) VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), server_id_str, name, description],
        )
        .map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT user_id FROM server_users WHERE server_id = ?1")
            .map_err(|e| e.to_string())?;
        let user_rows = stmt
            .query_map(params![server_id_str.clone()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        for user_row in user_rows {
            let user_id = user_row.map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR IGNORE INTO channel_users (channel_id, user_id) VALUES (?1, ?2)",
                params![id.to_string(), user_id],
            )
            .ok();
        }
        Ok(id)
    })
    .await
    .unwrap()
}

pub async fn db_create_channel_message(
    channel_id: Uuid,
    sent_by: Uuid,
    timestamp: i64,
    content: &str,
) -> Result<Uuid, String> {
    let channel_id = channel_id.to_string();
    let sent_by = sent_by.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO channel_messages (id, channel_id, sent_by, timestamp, content) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), channel_id, sent_by, timestamp, content],
        )
        .map_err(|e| e.to_string())?;
        Ok(id)
    })
    .await
    .unwrap()
}

pub async fn db_get_channel_messages(
    channel_id: Uuid,
    before: Option<i64>,
) -> Result<(Vec<ChannelMessage>, bool), String> {
    let channel_id_str = channel_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut messages: Vec<ChannelMessage> = Vec::new();
        
        // Use separate if/else blocks to avoid type conflicts
        if let Some(before_ts) = before {
            let mut stmt = conn.prepare(
                "SELECT id, sent_by, timestamp, content
                 FROM channel_messages
                 WHERE channel_id = ? AND timestamp < ?
                 ORDER BY timestamp DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![channel_id_str, before_ts], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, sent_by, timestamp, content) = row.map_err(|e| e.to_string())?;
                
                messages.push(ChannelMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    channel_id,
                    sent_by: Uuid::parse_str(&sent_by).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                });
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, sent_by, timestamp, content
                 FROM channel_messages
                 WHERE channel_id = ?
                 ORDER BY timestamp DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![channel_id_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, sent_by, timestamp, content) = row.map_err(|e| e.to_string())?;
                
                messages.push(ChannelMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    channel_id,
                    sent_by: Uuid::parse_str(&sent_by).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                });
            }
        }

        messages.reverse(); // Oldest first
        
        // Check if we've reached the oldest message
        let history_complete = if !messages.is_empty() {
            let oldest_ts = messages.first().unwrap().timestamp;
            let mut min_stmt = conn.prepare("SELECT MIN(timestamp) FROM channel_messages WHERE channel_id = ?")
                .map_err(|e| e.to_string())?;
            let min_ts: i64 = min_stmt.query_row(params![channel_id_str], |row| row.get(0))
                .unwrap_or(oldest_ts);
            oldest_ts <= min_ts
        } else {
            true
        };

        Ok((messages, history_complete))
    })
    .await
    .unwrap()
}

/// Get channel user list without profile images (for performance)
pub async fn db_get_channel_user_list_lightweight(channel_id: Uuid) -> Result<Vec<UserInfo>, String> {
    let channel_id_str = channel_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.color, u.role 
             FROM users u 
             JOIN channel_users cu ON u.id = cu.user_id 
             WHERE cu.channel_id = ?1 
             ORDER BY u.username"
        ).map_err(|e| e.to_string())?;

        let user_rows = stmt.query_map(params![channel_id_str], |row| {
            let role_str: String = row.get(3)?;
            Ok(UserInfo {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                username: row.get(1)?,
                color: parse_user_color(&row.get::<_, String>(2)?),
                role: match role_str.as_str() {
                    "Admin" => UserRole::Admin,
                    "Moderator" => UserRole::Moderator,
                    _ => UserRole::User,
                },
                status: UserStatus::Offline, // Default to offline, will be updated by server
            })
        }).map_err(|e| e.to_string())?;

        let mut users = Vec::new();
        for user_row in user_rows {
            users.push(user_row.map_err(|e| e.to_string())?);
        }

        Ok(users)
    })
    .await
    .unwrap()
}

pub async fn db_get_channel_user_list(channel_id: Uuid) -> Result<Vec<User>, String> {
    let channel_id_str = channel_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.color, u.role, u.profile_pic, u.cover_banner 
             FROM users u 
             JOIN channel_users cu ON u.id = cu.user_id 
             WHERE cu.channel_id = ?1"
        ).map_err(|e| e.to_string())?;

        let user_rows = stmt.query_map(params![channel_id_str], |row| {
            let role_str: String = row.get(3)?;
            Ok(User {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                username: row.get(1)?,
                color: parse_user_color(&row.get::<_, String>(2)?),
                role: match role_str.as_str() {
                    "Admin" => UserRole::Admin,
                    "Moderator" => UserRole::Moderator,
                    _ => UserRole::User,
                },
                profile_pic: row.get(4)?,
                cover_banner: row.get(5)?,
                status: UserStatus::Connected,
            })
        }).map_err(|e| e.to_string())?;

        let mut users = Vec::new();
        for user_row in user_rows {
            users.push(user_row.map_err(|e| e.to_string())?);
        }

        Ok(users)
    })
    .await
    .unwrap()
}

/// Enhanced channel message retrieval with optimized profile image handling
pub async fn db_get_channel_messages_by_timestamp(
    channel_id: Uuid,
    before: Option<i64>,
    limit: usize,
    reverse_order: bool,
) -> Result<(Vec<ChannelMessage>, bool), String> {
    let channel_id_str = channel_id.to_string();
    let limit = limit.min(200); // Safety limit

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        let mut messages = Vec::new();
        
        if let Some(before_ts) = before {
            let comparison = if reverse_order { ">=" } else { "<" };
            let order = if reverse_order { "DESC" } else { "ASC" };
            
            let query = format!(
                "SELECT id, sent_by, timestamp, content
                 FROM channel_messages
                 WHERE channel_id = ? AND timestamp {} ?
                 ORDER BY timestamp {} LIMIT ?",
                comparison, order
            );
            
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![channel_id_str, before_ts, limit + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, sent_by, timestamp, content) = 
                    row.map_err(|e| e.to_string())?;
                
                messages.push(ChannelMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    channel_id,
                    sent_by: Uuid::parse_str(&sent_by).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                });
            }
        } else {
            let order = if reverse_order { "DESC" } else { "ASC" };
            
            let query = format!(
                "SELECT id, sent_by, timestamp, content
                 FROM channel_messages
                 WHERE channel_id = ?
                 ORDER BY timestamp {} LIMIT ?",
                order
            );
            
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![channel_id_str, limit + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, sent_by, timestamp, content) = 
                    row.map_err(|e| e.to_string())?;
                
                messages.push(ChannelMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    channel_id,
                    sent_by: Uuid::parse_str(&sent_by).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                });
            }
        }

        // Check if we have more messages than requested
        let has_more = messages.len() > limit;
        if has_more {
            messages.pop(); // Remove the extra message
        }

        if reverse_order {
            messages.reverse();
        }

        Ok((messages, has_more))
    })
    .await
    .unwrap()
}

/// Get total message count for a channel (for pagination metadata)
pub async fn db_get_channel_message_count(channel_id: Uuid) -> Result<usize, String> {
    let channel_id_str = channel_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM channel_messages WHERE channel_id = ?"
        ).map_err(|e| e.to_string())?;
        
        let count: i64 = stmt.query_row(params![channel_id_str], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        
        Ok(count as usize)
    })
    .await
    .unwrap()
}

/// Get all channel IDs for a server
pub async fn db_get_server_channels(server_id: Uuid) -> Result<Vec<Uuid>, String> {
    let server_id_str = server_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT id FROM channels WHERE server_id = ?"
        ).map_err(|e| e.to_string())?;
        
        let rows = stmt.query_map(params![server_id_str], |row| {
            let id_str: String = row.get(0)?;
            Ok(Uuid::parse_str(&id_str).unwrap())
        }).map_err(|e| e.to_string())?;
        
        let mut channel_ids = Vec::new();
        for row in rows {
            channel_ids.push(row.map_err(|e| e.to_string())?);
        }
        
        Ok(channel_ids)
    })
    .await
    .unwrap()
}

/// Add user to a channel
pub async fn db_add_user_to_channel(channel_id: Uuid, user_id: Uuid) -> Result<(), String> {
    let channel_id_str = channel_id.to_string();
    let user_id_str = user_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        conn.execute(
            "INSERT OR IGNORE INTO channel_users (channel_id, user_id) VALUES (?1, ?2)",
            params![channel_id_str, user_id_str],
        ).map_err(|e| e.to_string())?;
        
        Ok(())
    })
    .await
    .unwrap()
}

/// Get users that share channels with the given user
pub async fn db_get_users_sharing_channels_with(user_id: Uuid) -> Result<Vec<Uuid>, String> {
    let user_id_str = user_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT DISTINCT cu2.user_id 
             FROM channel_users cu1 
             JOIN channel_users cu2 ON cu1.channel_id = cu2.channel_id 
             WHERE cu1.user_id = ? AND cu2.user_id != ?"
        ).map_err(|e| e.to_string())?;
        
        let rows = stmt.query_map(params![user_id_str, user_id_str], |row| {
            let id_str: String = row.get(0)?;
            Ok(Uuid::parse_str(&id_str).unwrap())
        }).map_err(|e| e.to_string())?;
        
        let mut user_ids = Vec::new();
        for row in rows {
            user_ids.push(row.map_err(|e| e.to_string())?);
        }
        
        Ok(user_ids)
    })
    .await
    .unwrap()
}
