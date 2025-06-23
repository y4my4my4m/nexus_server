use common::{DirectMessage, User, UserInfo, UserRole, UserStatus};
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
    let user1_id_str = user1_id.to_string();
    let user2_id_str = user2_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut messages: Vec<DirectMessage> = Vec::new();
        
        if let Some(before_ts) = before {
            let mut stmt = conn.prepare(
                "SELECT id, from_user_id, to_user_id, content, timestamp
                 FROM direct_messages 
                 WHERE ((from_user_id = ? AND to_user_id = ?) OR (from_user_id = ? AND to_user_id = ?)) 
                 AND timestamp < ?
                 ORDER BY timestamp DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![user1_id_str, user2_id_str, user2_id_str, user1_id_str, before_ts], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, from_user_id, to_user_id, content, timestamp) = 
                    row.map_err(|e| e.to_string())?;
                
                messages.push(DirectMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    from: Uuid::parse_str(&from_user_id).map_err(|e| e.to_string())?,
                    to: Uuid::parse_str(&to_user_id).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
                });
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, from_user_id, to_user_id, content, timestamp
                 FROM direct_messages 
                 WHERE ((from_user_id = ? AND to_user_id = ?) OR (from_user_id = ? AND to_user_id = ?))
                 ORDER BY timestamp DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![user1_id_str, user2_id_str, user2_id_str, user1_id_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, from_user_id, to_user_id, content, timestamp) = 
                    row.map_err(|e| e.to_string())?;
                
                messages.push(DirectMessage {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    from: Uuid::parse_str(&from_user_id).map_err(|e| e.to_string())?,
                    to: Uuid::parse_str(&to_user_id).map_err(|e| e.to_string())?,
                    timestamp,
                    content,
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
            let min_ts: i64 = min_stmt.query_row(params![user1_id_str, user2_id_str, user2_id_str, user1_id_str], |row| row.get(0))
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

/// Get DM user list without profile images (for better performance)
pub async fn db_get_dm_user_list_lightweight(user_id: Uuid) -> Result<Vec<UserInfo>, String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        // Get users we've had conversations with
        let mut stmt = conn.prepare(
            "SELECT DISTINCT 
                CASE 
                    WHEN dm.from_user_id = ? THEN dm.to_user_id 
                    ELSE dm.from_user_id 
                END as other_user_id
             FROM direct_messages dm 
             WHERE dm.from_user_id = ? OR dm.to_user_id = ?"
        ).map_err(|e| e.to_string())?;

        let user_id_rows = stmt.query_map(
            params![user_id_str, user_id_str, user_id_str], 
            |row| row.get::<_, String>(0)
        ).map_err(|e| e.to_string())?;

        let mut users = Vec::new();
        for user_id_row in user_id_rows {
            let other_user_id_str = user_id_row.map_err(|e| e.to_string())?;
            let other_user_id = Uuid::parse_str(&other_user_id_str).map_err(|e| e.to_string())?;
            
            // Get lightweight user profile
            let mut user_stmt = conn.prepare(
                "SELECT username, color, role FROM users WHERE id = ?"
            ).map_err(|e| e.to_string())?;
            
            if let Ok((username, color, role)) = user_stmt.query_row(
                params![other_user_id_str], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                }
            ) {
                users.push(UserInfo {
                    id: other_user_id,
                    username,
                    color: common::UserColor::new(color),
                    role: match role.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
                    status: UserStatus::Offline, // Default to offline, will be updated by server
                });
            }
        }

        Ok(users)
    })
    .await
    .unwrap()
}

pub async fn db_get_dm_user_list(user_id: Uuid) -> Result<Vec<User>, String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        // Get users we've had conversations with
        let mut stmt = conn.prepare(
            "SELECT DISTINCT 
                CASE 
                    WHEN dm.from_user_id = ? THEN dm.to_user_id 
                    ELSE dm.from_user_id 
                END as other_user_id
             FROM direct_messages dm 
             WHERE dm.from_user_id = ? OR dm.to_user_id = ?"
        ).map_err(|e| e.to_string())?;

        let user_id_rows = stmt.query_map(
            params![user_id_str, user_id_str, user_id_str], 
            |row| row.get::<_, String>(0)
        ).map_err(|e| e.to_string())?;

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
                    color: common::UserColor::new(color),
                    role: match role.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
                    profile_pic,
                    cover_banner,
                    status: UserStatus::Connected,
                });
            }
        }

        Ok(users)
    })
    .await
    .unwrap()
}

/// Enhanced direct message retrieval with optimized queries
pub async fn db_get_direct_messages_by_timestamp(
    user1_id: Uuid,
    user2_id: Uuid,
    before: Option<i64>,
    limit: usize,
    reverse_order: bool,
) -> Result<(Vec<DirectMessage>, bool), String> {
    let user1_id_str = user1_id.to_string();
    let user2_id_str = user2_id.to_string();
    let limit = limit.min(200); // Safety limit

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut messages = Vec::new();
        
        let base_query = 
            "SELECT id, from_user_id, to_user_id, timestamp, content
             FROM direct_messages
             WHERE ((from_user_id = ? AND to_user_id = ?) OR 
                    (from_user_id = ? AND to_user_id = ?))";
        
        let query = if let Some(_before_ts) = before {
            let comparison = if reverse_order { ">=" } else { "<" };
            let order = if reverse_order { "DESC" } else { "ASC" };
            format!("{} AND timestamp {} ? ORDER BY timestamp {} LIMIT ?", 
                    base_query, comparison, order)
        } else {
            let order = if reverse_order { "DESC" } else { "ASC" };
            format!("{} ORDER BY timestamp {} LIMIT ?", base_query, order)
        };
        
        let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
        
        let row_mapper = |row: &rusqlite::Row| -> rusqlite::Result<(String, String, String, i64, String)> {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        };
        
        let rows = if let Some(before_ts) = before {
            stmt.query_map(params![user1_id_str, user2_id_str, user2_id_str, user1_id_str, before_ts, limit + 1], row_mapper)
        } else {
            stmt.query_map(params![user1_id_str, user2_id_str, user2_id_str, user1_id_str, limit + 1], row_mapper)
        }.map_err(|e| e.to_string())?;

        for row in rows {
            let (id, from_user_id, to_user_id, timestamp, content) = 
                row.map_err(|e| e.to_string())?;
            
            messages.push(DirectMessage {
                id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                from: Uuid::parse_str(&from_user_id).map_err(|e| e.to_string())?,
                to: Uuid::parse_str(&to_user_id).map_err(|e| e.to_string())?,
                timestamp,
                content,
            });
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

/// Get total direct message count between two users (for pagination metadata)
pub async fn db_get_direct_message_count(user1_id: Uuid, user2_id: Uuid) -> Result<usize, String> {
    let user1_id_str = user1_id.to_string();
    let user2_id_str = user2_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM direct_messages 
             WHERE (from_user_id = ? AND to_user_id = ?) OR (from_user_id = ? AND to_user_id = ?)"
        ).map_err(|e| e.to_string())?;
        
        let count: i64 = stmt.query_row(params![user1_id_str, user2_id_str, user2_id_str, user1_id_str], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        
        Ok(count as usize)
    })
    .await
    .unwrap()
}
