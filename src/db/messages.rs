use crate::util::{parse_color, parse_user_color};
use common::{DirectMessage, User, UserRole, UserStatus};
use rusqlite::{params, Connection};
use tokio::task;
use uuid::Uuid;

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_store_direct_message(
    from_user_id: Uuid,
    to_user_id: Uuid,
    content: &str,
    timestamp: i64,
) -> Result<Uuid, String> {
    let from_user_id_str = from_user_id.to_string();
    let to_user_id_str = to_user_id.to_string();
    let content = content.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO direct_messages (id, from_user_id, to_user_id, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), from_user_id_str, to_user_id_str, content, timestamp],
        ).map_err(|e| e.to_string())?;

        Ok(id)
    })
    .await
    .unwrap()
}

pub async fn db_get_direct_messages(
    user1_id: Uuid,
    user2_id: Uuid,
    before: Option<i64>,
    _limit: usize,
) -> Result<(Vec<DirectMessage>, bool), String> {
    let user1_str = user1_id.to_string();
    let user2_str = user2_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut messages: Vec<DirectMessage> = Vec::new();
        
        // Use separate functions to avoid type conflicts
        if let Some(before_ts) = before {
            // Query with timestamp filter
            let mut stmt = conn.prepare(
                "SELECT dm.id, dm.from_user_id, dm.to_user_id, dm.content, dm.timestamp, u.username, u.color, u.profile_pic
                 FROM direct_messages dm
                 INNER JOIN users u ON dm.from_user_id = u.id
                 WHERE ((dm.from_user_id = ? AND dm.to_user_id = ?) OR (dm.from_user_id = ? AND dm.to_user_id = ?))
                 AND dm.timestamp < ?
                 ORDER BY dm.timestamp DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![user1_str, user2_str, user2_str, user1_str, before_ts], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, from_user_id, to_user_id, content, timestamp, username, color, profile_pic) = 
                    row.map_err(|e| e.to_string())?;
                
                messages.push(DirectMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    from: Uuid::parse_str(&from_user_id).map_err(|e| e.to_string())?,
                    to: Uuid::parse_str(&to_user_id).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                    author_username: username,
                    author_color: parse_user_color(&color),
                    author_profile_pic: profile_pic,
                });
            }
        } else {
            // Query without timestamp filter
            let mut stmt = conn.prepare(
                "SELECT dm.id, dm.from_user_id, dm.to_user_id, dm.content, dm.timestamp, u.username, u.color, u.profile_pic
                 FROM direct_messages dm
                 INNER JOIN users u ON dm.from_user_id = u.id
                 WHERE (dm.from_user_id = ? AND dm.to_user_id = ?) OR (dm.from_user_id = ? AND dm.to_user_id = ?)
                 ORDER BY dm.timestamp DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![user1_str, user2_str, user2_str, user1_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, from_user_id, to_user_id, content, timestamp, username, color, profile_pic) = 
                    row.map_err(|e| e.to_string())?;
                
                messages.push(DirectMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    from: Uuid::parse_str(&from_user_id).map_err(|e| e.to_string())?,
                    to: Uuid::parse_str(&to_user_id).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                    author_username: username,
                    author_color: parse_user_color(&color),
                    author_profile_pic: profile_pic,
                });
            }
        }

        messages.reverse(); // Oldest first
        
        // Check if we've reached the oldest message
        let history_complete = if !messages.is_empty() {
            let oldest_ts = messages.first().unwrap().timestamp;
            let mut min_stmt = conn.prepare(
                "SELECT MIN(timestamp) FROM direct_messages 
                 WHERE (from_user_id = ? AND to_user_id = ?) OR (from_user_id = ? AND to_user_id = ?)"
            ).map_err(|e| e.to_string())?;
            let min_ts: i64 = min_stmt.query_row(params![user1_str, user2_str, user2_str, user1_str], |row| row.get(0))
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

pub async fn db_get_dm_user_list(user_id: Uuid) -> Result<Vec<User>, String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        // Find all user IDs who have exchanged DMs with this user
        let mut stmt = conn.prepare(
            "SELECT DISTINCT 
                CASE 
                    WHEN from_user_id = ? THEN to_user_id 
                    ELSE from_user_id 
                END as other_user_id
             FROM direct_messages 
             WHERE from_user_id = ? OR to_user_id = ?"
        ).map_err(|e| e.to_string())?;

        let user_id_rows = stmt.query_map(params![user_id_str, user_id_str, user_id_str], |row| {
            row.get::<_, String>(0)
        }).map_err(|e| e.to_string())?;

        let mut users = Vec::new();
        for user_id_row in user_id_rows {
            let other_user_id_str = user_id_row.map_err(|e| e.to_string())?;
            let other_user_id = Uuid::parse_str(&other_user_id_str).map_err(|e| e.to_string())?;
            
            // Get user profile
            let mut user_stmt = conn.prepare(
                "SELECT username, color, role, profile_pic, cover_banner FROM users WHERE id = ?"
            ).map_err(|e| e.to_string())?;
            
            if let Ok((username, color, role, profile_pic, cover_banner)) = user_stmt.query_row(
                params![other_user_id_str], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                }
            ) {
                users.push(User {
                    id: other_user_id,
                    username,
                    color: parse_user_color(&color),
                    role: match role.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
                    profile_pic,
                    cover_banner,
                    status: UserStatus::Offline, // Will be updated by service layer
                });
            }
        }

        Ok(users)
    })
    .await
    .unwrap()
}

/// Enhanced direct message retrieval with timestamp-based pagination
pub async fn db_get_direct_messages_by_timestamp(
    user1_id: Uuid,
    user2_id: Uuid,
    before: Option<i64>,
    limit: usize,
    reverse_order: bool,
) -> Result<(Vec<DirectMessage>, bool), String> {
    let user1_str = user1_id.to_string();
    let user2_str = user2_id.to_string();
    let limit = limit.min(200); // Safety limit

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut all_messages = Vec::new();
        
        if let Some(before_ts) = before {
            let order = if reverse_order { "DESC" } else { "ASC" };
            let comparison = if reverse_order { "<" } else { ">" };
            
            let query = format!(
                "SELECT dm.id, dm.from_user_id, dm.to_user_id, dm.content, dm.timestamp, u.username, u.color, u.profile_pic
                 FROM direct_messages dm
                 INNER JOIN users u ON dm.from_user_id = u.id
                 WHERE ((dm.from_user_id = ? AND dm.to_user_id = ?) OR (dm.from_user_id = ? AND dm.to_user_id = ?))
                 AND dm.timestamp {} ?
                 ORDER BY dm.timestamp {} LIMIT ?",
                comparison, order
            );
            
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![user1_str, user2_str, user2_str, user1_str, before_ts, limit + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, from_user_id, to_user_id, content, timestamp, username, color, profile_pic) = 
                    row.map_err(|e| e.to_string())?;
                
                all_messages.push(DirectMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    from: Uuid::parse_str(&from_user_id).map_err(|e| e.to_string())?,
                    to: Uuid::parse_str(&to_user_id).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                    author_username: username,
                    author_color: parse_user_color(&color),
                    author_profile_pic: profile_pic,
                });
            }
        } else {
            let order = if reverse_order { "DESC" } else { "ASC" };
            
            let query = format!(
                "SELECT dm.id, dm.from_user_id, dm.to_user_id, dm.content, dm.timestamp, u.username, u.color, u.profile_pic
                 FROM direct_messages dm
                 INNER JOIN users u ON dm.from_user_id = u.id
                 WHERE (dm.from_user_id = ? AND dm.to_user_id = ?) OR (dm.from_user_id = ? AND dm.to_user_id = ?)
                 ORDER BY dm.timestamp {} LIMIT ?",
                order
            );
            
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![user1_str, user2_str, user2_str, user1_str, limit + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, from_user_id, to_user_id, content, timestamp, username, color, profile_pic) = 
                    row.map_err(|e| e.to_string())?;
                
                all_messages.push(DirectMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    from: Uuid::parse_str(&from_user_id).map_err(|e| e.to_string())?,
                    to: Uuid::parse_str(&to_user_id).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                    author_username: username,
                    author_color: parse_user_color(&color),
                    author_profile_pic: profile_pic,
                });
            }
        }

        // Check if there are more messages
        let has_more = all_messages.len() > limit;
        if has_more {
            all_messages.pop(); // Remove the extra message
        }

        // If we fetched in reverse order, reverse again to get chronological order
        if reverse_order {
            all_messages.reverse();
        }

        Ok((all_messages, has_more))
    })
    .await
    .unwrap()
}

/// Get total DM count between two users (for pagination metadata)
pub async fn db_get_direct_message_count(user1_id: Uuid, user2_id: Uuid) -> Result<usize, String> {
    let user1_str = user1_id.to_string();
    let user2_str = user2_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM direct_messages 
             WHERE (from_user_id = ? AND to_user_id = ?) OR (from_user_id = ? AND to_user_id = ?)"
        ).map_err(|e| e.to_string())?;
        
        let count: i64 = stmt.query_row(params![user1_str, user2_str, user2_str, user1_str], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        
        Ok(count as usize)
    })
    .await
    .unwrap()
}
