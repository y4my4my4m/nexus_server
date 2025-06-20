// server/src/db/channels.rs
// Channel DB functions

use crate::util::parse_color;
use common::{ChannelMessage, User, UserRole, UserStatus};
use rusqlite::{params, Connection};
use tokio::task;
use uuid::Uuid;

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_create_channel(
    server_id: Uuid,
    name: &str,
    description: &str,
) -> Result<Uuid, String> {
    let server_id_str = server_id.to_string();
    let name = name.to_string();
    let description = description.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
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
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
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

pub async fn db_get_channel_messages(channel_id: Uuid, before: Option<Uuid>) -> Result<(Vec<ChannelMessage>, bool), String> {
    let channel_id_str = channel_id.to_string();
    let before_ts: Option<i64> = None; // You can add pagination support if needed
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut query = String::from("SELECT m.id, m.channel_id, m.sent_by, m.timestamp, m.content, u.username, u.color, u.profile_pic FROM channel_messages m INNER JOIN users u ON m.sent_by = u.id WHERE m.channel_id = ?1");
        let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&channel_id_str];
        if let Some(_before) = before {
            // If you want to support pagination by message id, you can add logic here
        }
        query.push_str(" ORDER BY m.timestamp DESC LIMIT 50");
        let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(&params_vec[..], |row| {
            Ok(ChannelMessage {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                channel_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap(),
                sent_by: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap(),
                timestamp: row.get(3)?,
                content: row.get(4)?,
                author_username: row.get(5)?,
                author_color: crate::util::parse_color(&row.get::<_, String>(6)?),
                author_profile_pic: row.get(7).ok(),
            })
        }).map_err(|e| e.to_string())?;
        let mut messages: Vec<ChannelMessage> = Vec::new();
        for row in rows {
            messages.push(row.map_err(|e| e.to_string())?);
        }
        messages.reverse(); // Oldest first
        Ok((messages, true))
    }).await.unwrap()
}

pub async fn db_get_channel_user_list(channel_id: Uuid) -> Result<Vec<User>, String> {
    let channel_id_str = channel_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.color, u.role, u.profile_pic, u.cover_banner 
             FROM users u 
             INNER JOIN channel_users cu ON u.id = cu.user_id 
             WHERE cu.channel_id = ?1 
             ORDER BY u.username"
        ).map_err(|e| e.to_string())?;

        let user_rows = stmt.query_map(params![channel_id_str], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        }).map_err(|e| e.to_string())?;

        let mut users = Vec::new();
        for user_row in user_rows {
            let (id, username, color, role, profile_pic, cover_banner) = user_row.map_err(|e| e.to_string())?;
            
            users.push(User {
                id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                username,
                color: parse_color(&color),
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

        Ok(users)
    })
    .await
    .unwrap()
}

pub async fn get_users_sharing_channels_with(user_id: Uuid) -> Result<Vec<Uuid>, String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT DISTINCT cu2.user_id 
             FROM channel_users cu1 
             INNER JOIN channel_users cu2 ON cu1.channel_id = cu2.channel_id 
             WHERE cu1.user_id = ?1 AND cu2.user_id != ?1"
        ).map_err(|e| e.to_string())?;

        let user_rows = stmt.query_map(params![user_id_str], |row| {
            row.get::<_, String>(0)
        }).map_err(|e| e.to_string())?;

        let mut user_ids = Vec::new();
        for user_row in user_rows {
            let user_id_str = user_row.map_err(|e| e.to_string())?;
            user_ids.push(Uuid::parse_str(&user_id_str).map_err(|e| e.to_string())?);
        }

        Ok(user_ids)
    })
    .await
    .unwrap()
}

pub async fn get_server_channels(server_id: Uuid) -> Result<Vec<Uuid>, String> {
    let server_id_str = server_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare("SELECT id FROM channels WHERE server_id = ?1")
            .map_err(|e| e.to_string())?;

        let channel_rows = stmt.query_map(params![server_id_str], |row| {
            row.get::<_, String>(0)
        }).map_err(|e| e.to_string())?;

        let mut channel_ids = Vec::new();
        for channel_row in channel_rows {
            let channel_id_str = channel_row.map_err(|e| e.to_string())?;
            channel_ids.push(Uuid::parse_str(&channel_id_str).map_err(|e| e.to_string())?);
        }

        Ok(channel_ids)
    })
    .await
    .unwrap()
}

pub async fn add_user_to_channel(channel_id: Uuid, user_id: Uuid) -> Result<(), String> {
    let channel_id_str = channel_id.to_string();
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT OR IGNORE INTO channel_users (channel_id, user_id) VALUES (?1, ?2)",
            params![channel_id_str, user_id_str],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}
