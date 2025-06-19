// server/src/db/notifications.rs
// Notification DB functions

use uuid::Uuid;
use rusqlite::{params, Connection};
use common::{Notification, NotificationType};

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_insert_notification(user_id: Uuid, notif_type: &str, related_id: Uuid, extra: Option<String>) -> Result<(), String> {
    let user_id = user_id.to_string();
    let notif_type = notif_type.to_string();
    let related_id = related_id.to_string();
    let extra = extra.unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO notifications (id, user_id, type, related_id, created_at, read, extra) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
            params![id, user_id, notif_type, related_id, now, extra],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

pub async fn db_get_notifications(user_id: Uuid, before: Option<i64>) -> Result<(Vec<Notification>, bool), String> {
    let user_id = user_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt;
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(user_id.clone())];
        if let Some(before_ts) = before {
            stmt = conn.prepare("SELECT id, type, related_id, created_at, read, extra FROM notifications WHERE user_id = ?1 AND created_at < ?2 ORDER BY created_at DESC LIMIT 50").map_err(|e| e.to_string())?;
            params_vec.push(Box::new(before_ts));
        } else {
            stmt = conn.prepare("SELECT id, type, related_id, created_at, read, extra FROM notifications WHERE user_id = ?1 ORDER BY created_at DESC LIMIT 50").map_err(|e| e.to_string())?;
        }
        let rows: Box<dyn Iterator<Item = _>> = if params_vec.len() == 2 {
            Box::new(stmt.query_map(params![params_vec[0].as_ref(), params_vec[1].as_ref()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, i32>(4)?, row.get::<_, Option<String>>(5)?))
            }).unwrap())
        } else {
            Box::new(stmt.query_map(params![params_vec[0].as_ref()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, i32>(4)?, row.get::<_, Option<String>>(5)?))
            }).unwrap())
        };
        let mut notifications = Vec::new();
        for row in rows {
            let (id, notif_type, related_id, created_at, read, extra) = row.map_err(|e| e.to_string())?;
            let notif_type = match notif_type.as_str() {
                "ThreadReply" => NotificationType::ThreadReply,
                "DM" => NotificationType::DM,
                "Announcement" => NotificationType::Announcement,
                "Mention" => NotificationType::Mention,
                other => NotificationType::Other(other.to_string()),
            };
            notifications.push(Notification {
                id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                user_id: Uuid::parse_str(&user_id).map_err(|e| e.to_string())?,
                notif_type,
                related_id: Uuid::parse_str(&related_id).map_err(|e| e.to_string())?,
                created_at,
                read: read != 0,
                extra,
            });
        }
        notifications.reverse();
        let history_complete = notifications.len() < 50;
        Ok((notifications, history_complete))
    }).await.unwrap()
}

pub async fn db_mark_notification_read(notification_id: Uuid) -> Result<(), String> {
    let notification_id = notification_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE notifications SET read = 1 WHERE id = ?1",
            params![notification_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}
