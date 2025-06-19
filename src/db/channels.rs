// server/src/db/channels.rs
// Channel DB functions

use common::{ChannelMessage, User};
use rusqlite::{params, Connection};
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
    // TODO: Implement real logic. For now, return empty/default.
    Ok((Vec::new(), true))
}

pub async fn db_get_channel_user_list(channel_id: Uuid) -> Result<Vec<User>, String> {
    // TODO: Implement real logic. For now, return empty/default.
    Ok(Vec::new())
}
