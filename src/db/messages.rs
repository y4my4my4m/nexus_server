// server/src/db/messages.rs
// Message DB functions (DMs, channel messages)

use uuid::Uuid;
use rusqlite::{params, Connection};
use crate::db::users::db_get_user_by_id;
use common::{User, DirectMessage, UserStatus};

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_store_direct_message(from_user_id: Uuid, to_user_id: Uuid, content: &str, timestamp: i64) -> Result<Uuid, String> {
    let from_user_id = from_user_id.to_string();
    let to_user_id = to_user_id.to_string();
    let content = content.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO direct_messages (id, from_user_id, to_user_id, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), from_user_id, to_user_id, content, timestamp],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }).await.unwrap()
}

pub async fn db_get_dm_user_list(user_id: Uuid) -> Result<Vec<User>, String> {
    use std::collections::HashSet;
    let user_id_str = user_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT from_user_id, to_user_id FROM direct_messages WHERE from_user_id = ?1 OR to_user_id = ?1").map_err(|e| e.to_string())?;
        let mut user_ids = HashSet::new();
        let rows = stmt.query_map(params![user_id_str.clone()], |row| {
            let from: String = row.get(0)?;
            let to: String = row.get(1)?;
            Ok((from, to))
        }).map_err(|e| e.to_string())?;
        for row in rows {
            let (from, to) = row.map_err(|e| e.to_string())?;
            if from != user_id_str { user_ids.insert(from); }
            if to != user_id_str { user_ids.insert(to); }
        }
        let mut users = Vec::new();
        for uid in user_ids {
            let uuid = Uuid::parse_str(&uid).map_err(|e| e.to_string())?;
            if let Ok(profile) = futures::executor::block_on(db_get_user_by_id(uuid)) {
                users.push(User {
                    id: profile.id,
                    username: profile.username,
                    color: profile.color,
                    role: profile.role,
                    profile_pic: profile.profile_pic.clone(),
                    cover_banner: profile.cover_banner.clone(),
                    status: UserStatus::Offline,
                });
            }
        }
        Ok(users)
    }).await.unwrap()
}

pub async fn db_get_direct_messages(user1: Uuid, user2: Uuid, before: Option<i64>) -> Result<(Vec<DirectMessage>, bool), String> {
    let user1_str = user1.to_string();
    let user2_str = user2.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut msgs = Vec::new();
        let raw_results: Vec<(String, String, String, String, i64)> = if let Some(before_ts) = before {
            let mut stmt = conn.prepare("SELECT id, from_user_id, to_user_id, content, timestamp FROM direct_messages WHERE ((from_user_id = ?1 AND to_user_id = ?2) OR (from_user_id = ?2 AND to_user_id = ?1)) AND timestamp < ?3 ORDER BY timestamp DESC LIMIT 20").map_err(|e| e.to_string())?;
            let rows_iter = stmt.query_map(params![user1_str.clone(), user2_str.clone(), before_ts], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?))
            }).map_err(|e| e.to_string())?;
            let mut results = Vec::new();
            for row in rows_iter {
                results.push(row.map_err(|e| e.to_string())?);
            }
            results
        } else {
            let mut stmt = conn.prepare("SELECT id, from_user_id, to_user_id, content, timestamp FROM direct_messages WHERE (from_user_id = ?1 AND to_user_id = ?2) OR (from_user_id = ?2 AND to_user_id = ?1) ORDER BY timestamp DESC LIMIT 20").map_err(|e| e.to_string())?;
            let rows_iter = stmt.query_map(params![user1_str.clone(), user2_str.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?))
            }).map_err(|e| e.to_string())?;
            let mut results = Vec::new();
            for row in rows_iter {
                results.push(row.map_err(|e| e.to_string())?);
            }
            results
        };
        for (id, from, to, content, timestamp) in raw_results {
            let from_uuid = Uuid::parse_str(&from).map_err(|e| e.to_string())?;
            let author_profile = {
                let mut profile_stmt = conn.prepare("SELECT id, username, password_hash, color, role, bio, url1, url2, url3, location, profile_pic, cover_banner FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
                profile_stmt.query_row(params![from], |row| {
                    let username: String = row.get(1)?;
                    let color: String = row.get(3)?;
                    let profile_pic: Option<String> = row.get(10)?;
                    Ok((username, color, profile_pic))
                }).map_err(|_| "User not found".to_string())?
            };
            msgs.push(DirectMessage {
                id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                from: from_uuid,
                to: Uuid::parse_str(&to).map_err(|e| e.to_string())?,
                timestamp,
                content,
                author_username: author_profile.0,
                author_color: crate::util::parse_color(&author_profile.1),
                author_profile_pic: author_profile.2,
            });
        }
        msgs.reverse();
        let history_complete = if !msgs.is_empty() {
            let oldest_ts = msgs.first().unwrap().timestamp;
            let mut min_stmt = conn.prepare("SELECT MIN(timestamp) FROM direct_messages WHERE (from_user_id = ?1 AND to_user_id = ?2) OR (from_user_id = ?2 AND to_user_id = ?1)").map_err(|e| e.to_string())?;
            let min_ts: i64 = min_stmt.query_row(params![user1_str.clone(), user2_str.clone()], |row| row.get(0)).unwrap_or(oldest_ts);
            oldest_ts <= min_ts
        } else {
            true
        };
        Ok((msgs, history_complete))
    }).await.unwrap()
}
