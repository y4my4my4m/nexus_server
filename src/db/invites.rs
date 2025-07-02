use crate::db::db_config;
use crate::errors::{Result, ServerError};
use nexus_tui_common::{ServerInvite, ServerInviteStatus, User, Server};
use rusqlite::{params, Connection};
use uuid::Uuid;
use std::str::FromStr;

pub async fn db_create_server_invite(
    from_user_id: Uuid,
    to_user_id: Uuid,
    server_id: Uuid,
) -> Result<Uuid> {
    let invite_id = Uuid::new_v4();
    let timestamp = chrono::Utc::now().timestamp();
    
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path())?;
        conn.execute(
            "INSERT INTO server_invites (id, from_user_id, to_user_id, server_id, timestamp, status) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                invite_id.to_string(),
                from_user_id.to_string(),
                to_user_id.to_string(),
                server_id.to_string(),
                timestamp,
                "Pending"
            ],
        )?;
        Ok::<Uuid, rusqlite::Error>(invite_id)
    })
    .await
    .map_err(|e| ServerError::Internal(e.to_string()))?
    .map_err(|e| ServerError::Database(e.to_string()))
}

pub async fn db_get_pending_invites_for_user(user_id: Uuid) -> Result<Vec<ServerInvite>> {
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path())?;
        let mut stmt = conn.prepare(
            "SELECT si.id, si.from_user_id, si.to_user_id, si.server_id, si.timestamp, si.status,
                    u.username, u.color, u.role, u.profile_pic, u.cover_banner,
                    s.name, s.description, s.public, s.invite_code, s.icon, s.banner, s.owner
             FROM server_invites si
             JOIN users u ON si.from_user_id = u.id
             JOIN servers s ON si.server_id = s.id
             WHERE si.to_user_id = ?1 AND si.status = 'Pending'
             ORDER BY si.timestamp DESC"
        )?;
        
        let invite_iter = stmt.query_map(params![user_id.to_string()], |row| {
            let status_str: String = row.get(5)?;
            let status = match status_str.as_str() {
                "Pending" => ServerInviteStatus::Pending,
                "Accepted" => ServerInviteStatus::Accepted,
                "Declined" => ServerInviteStatus::Declined,
                "Expired" => ServerInviteStatus::Expired,
                _ => ServerInviteStatus::Pending,
            };
            
            let color_str: String = row.get(7)?;
            let color = crate::util::parse_color(&color_str);
            
            let role_str: String = row.get(8)?;
            let role = crate::util::parse_role(&role_str);
            
            let from_user = User {
                id: Uuid::from_str(&row.get::<_, String>(1)?).unwrap(),
                username: row.get(6)?,
                color: color.into(),
                role,
                profile_pic: row.get(9)?,
                cover_banner: row.get(10)?,
                status: nexus_tui_common::UserStatus::Connected,
            };
            
            let server = Server {
                id: Uuid::from_str(&row.get::<_, String>(3)?).unwrap(),
                name: row.get(11)?,
                description: row.get(12)?,
                public: row.get::<_, i32>(13)? != 0,
                invite_code: row.get(14)?,
                icon: row.get(15)?,
                banner: row.get(16)?,
                owner: Uuid::from_str(&row.get::<_, String>(17)?).unwrap(),
                mods: vec![], // We'll populate this separately if needed
                userlist: vec![], // We'll populate this separately if needed
                channels: vec![], // We'll populate this separately if needed
            };
            
            Ok(ServerInvite {
                id: Uuid::from_str(&row.get::<_, String>(0)?).unwrap(),
                from_user,
                to_user_id: Uuid::from_str(&row.get::<_, String>(2)?).unwrap(),
                server,
                timestamp: row.get(4)?,
                status,
            })
        })?;
        
        let mut invites = Vec::new();
        for invite in invite_iter {
            invites.push(invite?);
        }
        
        Ok::<Vec<ServerInvite>, rusqlite::Error>(invites)
    })
    .await
    .map_err(|e| ServerError::Internal(e.to_string()))?
    .map_err(|e| ServerError::Database(e.to_string()))
}

pub async fn db_update_invite_status(invite_id: Uuid, status: ServerInviteStatus) -> Result<()> {
    let status_str = match status {
        ServerInviteStatus::Pending => "Pending",
        ServerInviteStatus::Accepted => "Accepted",
        ServerInviteStatus::Declined => "Declined",
        ServerInviteStatus::Expired => "Expired",
    };
    
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path())?;
        conn.execute(
            "UPDATE server_invites SET status = ?1 WHERE id = ?2",
            params![status_str, invite_id.to_string()],
        )?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .map_err(|e| ServerError::Internal(e.to_string()))?
    .map_err(|e| ServerError::Database(e.to_string()))
}

pub async fn db_get_invite_by_id(invite_id: Uuid) -> Result<Option<ServerInvite>> {
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path())?;
        let mut stmt = conn.prepare(
            "SELECT si.id, si.from_user_id, si.to_user_id, si.server_id, si.timestamp, si.status,
                    u.username, u.color, u.role, u.profile_pic, u.cover_banner,
                    s.name, s.description, s.public, s.invite_code, s.icon, s.banner, s.owner
             FROM server_invites si
             JOIN users u ON si.from_user_id = u.id
             JOIN servers s ON si.server_id = s.id
             WHERE si.id = ?1"
        )?;
        
        let mut invite_iter = stmt.query_map(params![invite_id.to_string()], |row| {
            let status_str: String = row.get(5)?;
            let status = match status_str.as_str() {
                "Pending" => ServerInviteStatus::Pending,
                "Accepted" => ServerInviteStatus::Accepted,
                "Declined" => ServerInviteStatus::Declined,
                "Expired" => ServerInviteStatus::Expired,
                _ => ServerInviteStatus::Pending,
            };
            
            let color_str: String = row.get(7)?;
            let color = crate::util::parse_color(&color_str);
            
            let role_str: String = row.get(8)?;
            let role = crate::util::parse_role(&role_str);
            
            let from_user = User {
                id: Uuid::from_str(&row.get::<_, String>(1)?).unwrap(),
                username: row.get(6)?,
                color: color.into(),
                role,
                profile_pic: row.get(9)?,
                cover_banner: row.get(10)?,
                status: nexus_tui_common::UserStatus::Connected,
            };
            
            let server = Server {
                id: Uuid::from_str(&row.get::<_, String>(3)?).unwrap(),
                name: row.get(11)?,
                description: row.get(12)?,
                public: row.get::<_, i32>(13)? != 0,
                invite_code: row.get(14)?,
                icon: row.get(15)?,
                banner: row.get(16)?,
                owner: Uuid::from_str(&row.get::<_, String>(17)?).unwrap(),
                mods: vec![],
                userlist: vec![],
                channels: vec![],
            };
            
            Ok(ServerInvite {
                id: Uuid::from_str(&row.get::<_, String>(0)?).unwrap(),
                from_user,
                to_user_id: Uuid::from_str(&row.get::<_, String>(2)?).unwrap(),
                server,
                timestamp: row.get(4)?,
                status,
            })
        })?;
        
        if let Some(invite_result) = invite_iter.next() {
            Ok::<Option<ServerInvite>, rusqlite::Error>(Some(invite_result?))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| ServerError::Internal(e.to_string()))?
    .map_err(|e| ServerError::Database(e.to_string()))
}

pub async fn db_check_existing_invite(
    from_user_id: Uuid, 
    to_user_id: Uuid, 
    server_id: Uuid
) -> Result<bool> {
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path())?;
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM server_invites 
             WHERE from_user_id = ?1 AND to_user_id = ?2 AND server_id = ?3 AND status = 'Pending'"
        )?;
        
        let count: i64 = stmt.query_row(
            params![
                from_user_id.to_string(),
                to_user_id.to_string(),
                server_id.to_string()
            ],
            |row| row.get(0)
        )?;
        
        Ok::<bool, rusqlite::Error>(count > 0)
    })
    .await
    .map_err(|e| ServerError::Internal(e.to_string()))?
    .map_err(|e| ServerError::Database(e.to_string()))
}

pub async fn db_get_pending_invite_from_user(
    from_user_id: Uuid,
    to_user_id: Uuid,
) -> Result<Option<ServerInvite>> {
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path())?;
        let mut stmt = conn.prepare(
            "SELECT si.id, si.from_user_id, si.to_user_id, si.server_id, si.timestamp, si.status,
                    u.username, u.color, u.role, u.profile_pic, u.cover_banner,
                    s.name, s.description, s.public, s.invite_code, s.icon, s.banner, s.owner
             FROM server_invites si
             JOIN users u ON si.from_user_id = u.id
             JOIN servers s ON si.server_id = s.id
             WHERE si.from_user_id = ?1 AND si.to_user_id = ?2 AND si.status = 'Pending'
             ORDER BY si.timestamp DESC
             LIMIT 1"
        )?;
        
        let mut invite_iter = stmt.query_map(params![from_user_id.to_string(), to_user_id.to_string()], |row| {
            let status_str: String = row.get(5)?;
            let status = match status_str.as_str() {
                "Pending" => ServerInviteStatus::Pending,
                "Accepted" => ServerInviteStatus::Accepted,
                "Declined" => ServerInviteStatus::Declined,
                "Expired" => ServerInviteStatus::Expired,
                _ => ServerInviteStatus::Pending,
            };
            
            let color_str: String = row.get(7)?;
            let color = crate::util::parse_color(&color_str);
            
            let role_str: String = row.get(8)?;
            let role = crate::util::parse_role(&role_str);
            
            let from_user = User {
                id: Uuid::from_str(&row.get::<_, String>(1)?).unwrap(),
                username: row.get(6)?,
                color: color.into(),
                role,
                profile_pic: row.get(9)?,
                cover_banner: row.get(10)?,
                status: nexus_tui_common::UserStatus::Connected,
            };
            
            let server = Server {
                id: Uuid::from_str(&row.get::<_, String>(3)?).unwrap(),
                name: row.get(11)?,
                description: row.get(12)?,
                public: row.get::<_, i32>(13)? != 0,
                invite_code: row.get(14)?,
                icon: row.get(15)?,
                banner: row.get(16)?,
                owner: Uuid::from_str(&row.get::<_, String>(17)?).unwrap(),
                mods: vec![],
                userlist: vec![],
                channels: vec![],
            };
            
            Ok(ServerInvite {
                id: Uuid::from_str(&row.get::<_, String>(0)?).unwrap(),
                from_user,
                to_user_id: Uuid::from_str(&row.get::<_, String>(2)?).unwrap(),
                server,
                timestamp: row.get(4)?,
                status,
            })
        })?;
        
        if let Some(invite_result) = invite_iter.next() {
            Ok::<Option<ServerInvite>, rusqlite::Error>(Some(invite_result?))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|e| ServerError::Internal(e.to_string()))?
    .map_err(|e| ServerError::Database(e.to_string()))
}