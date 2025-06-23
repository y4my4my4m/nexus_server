use crate::db::db_config;
use common::{Notification, NotificationType};
use rusqlite::{params, Connection};
use tokio::task;
use uuid::Uuid;

pub async fn db_insert_notification(
    user_id: Uuid,
    notif_type: &str,
    related_id: Uuid,
    extra: Option<String>,
) -> Result<(), String> {
    let user_id_str = user_id.to_string();
    let notif_type = notif_type.to_string();
    let related_id_str = related_id.to_string();
    let now = chrono::Utc::now().timestamp();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO notifications (id, user_id, type, related_id, created_at, read, extra) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
            params![id.to_string(), user_id_str, notif_type, related_id_str, now, extra],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_get_notifications(
    user_id: Uuid,
    before: Option<i64>,
) -> Result<(Vec<Notification>, bool), String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut notifications = Vec::new();
        
        // Use separate if/else blocks to avoid type conflicts
        if let Some(before_ts) = before {
            let mut stmt = conn.prepare(
                "SELECT id, type, related_id, created_at, read, extra 
                 FROM notifications 
                 WHERE user_id = ? AND created_at < ? 
                 ORDER BY created_at DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![user_id_str, before_ts], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i32>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, notif_type, related_id, created_at, read, extra) = row.map_err(|e| e.to_string())?;
                
                let notification_type = match notif_type.as_str() {
                    "ThreadReply" => NotificationType::ThreadReply,
                    "DM" => NotificationType::DM,
                    "Announcement" => NotificationType::Announcement,
                    "Mention" => NotificationType::Mention,
                    other => NotificationType::Other(other.to_string()),
                };
                
                notifications.push(Notification {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    user_id: Uuid::parse_str(&user_id_str).map_err(|e| e.to_string())?,
                    notif_type: notification_type,
                    related_id: Uuid::parse_str(&related_id).map_err(|e| e.to_string())?,
                    created_at,
                    read: read != 0,
                    extra,
                });
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, type, related_id, created_at, read, extra 
                 FROM notifications 
                 WHERE user_id = ? 
                 ORDER BY created_at DESC LIMIT 50"
            ).map_err(|e| e.to_string())?;
            
            let rows = stmt.query_map(params![user_id_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i32>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            }).map_err(|e| e.to_string())?;

            for row in rows {
                let (id, notif_type, related_id, created_at, read, extra) = row.map_err(|e| e.to_string())?;
                
                let notification_type = match notif_type.as_str() {
                    "ThreadReply" => NotificationType::ThreadReply,
                    "DM" => NotificationType::DM,
                    "Announcement" => NotificationType::Announcement,
                    "Mention" => NotificationType::Mention,
                    other => NotificationType::Other(other.to_string()),
                };
                
                notifications.push(Notification {
                    id: Uuid::parse_str(&id).map_err(|e| e.to_string())?,
                    user_id: Uuid::parse_str(&user_id_str).map_err(|e| e.to_string())?,
                    notif_type: notification_type,
                    related_id: Uuid::parse_str(&related_id).map_err(|e| e.to_string())?,
                    created_at,
                    read: read != 0,
                    extra,
                });
            }
        }

        notifications.reverse(); // Oldest first
        let history_complete = notifications.len() < 50;

        Ok((notifications, history_complete))
    })
    .await
    .unwrap()
}

pub async fn db_mark_notification_read(notification_id: Uuid) -> Result<(), String> {
    let notification_id_str = notification_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE notifications SET read = 1 WHERE id = ?1",
            params![notification_id_str],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}
