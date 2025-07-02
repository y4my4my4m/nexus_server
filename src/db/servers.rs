use crate::db::db_config;
use nexus_tui_common::Server;
use rusqlite::{params, Connection};
use tokio::task;
use uuid::Uuid;

pub async fn db_create_server(
    name: &str,
    description: &str,
    public: bool,
    owner: Uuid,
    icon: Option<&str>,
    banner: Option<&str>,
) -> Result<Uuid, String> {
    let name = name.to_string();
    let description = description.to_string();
    let icon = icon.map(|s| s.to_string());
    let banner = banner.map(|s| s.to_string());
    let owner = owner.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO servers (id, name, description, public, owner, icon, banner) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id.to_string(), name, description, public as i32, owner, icon, banner],
        ).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO server_users (server_id, user_id) VALUES (?1, ?2)",
            params![id.to_string(), owner],
        ).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO server_mods (server_id, user_id) VALUES (?1, ?2)",
            params![id.to_string(), owner],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }).await.unwrap()
}

pub async fn db_get_user_servers(user_id: Uuid) -> Result<Vec<Server>, String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.description, s.public, s.invite_code, s.icon, s.banner, s.owner 
             FROM servers s 
             INNER JOIN server_users su ON s.id = su.server_id 
             WHERE su.user_id = ?1"
        ).map_err(|e| e.to_string())?;

        let server_rows = stmt.query_map(params![user_id_str], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
            ))
        }).map_err(|e| e.to_string())?;

        let mut servers = Vec::new();
        for server_row in server_rows {
            let (id, name, description, public, invite_code, icon, banner, owner) = 
                server_row.map_err(|e| e.to_string())?;
            
            let server_id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
            
            // Get moderators
            let mut mods_stmt = conn.prepare("SELECT user_id FROM server_mods WHERE server_id = ?1")
                .map_err(|e| e.to_string())?;
            let mods: Vec<Uuid> = mods_stmt.query_map(params![id], |row| {
                let user_id_str: String = row.get(0)?;
                Ok(Uuid::parse_str(&user_id_str).unwrap())
            }).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;

            // Get userlist
            let mut users_stmt = conn.prepare("SELECT user_id FROM server_users WHERE server_id = ?1")
                .map_err(|e| e.to_string())?;
            let userlist: Vec<Uuid> = users_stmt.query_map(params![id], |row| {
                let user_id_str: String = row.get(0)?;
                Ok(Uuid::parse_str(&user_id_str).unwrap())
            }).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;

            // Get channels (simplified - just metadata without messages)
            let mut channels_stmt = conn.prepare(
                "SELECT id, name, description FROM channels WHERE server_id = ?1"
            ).map_err(|e| e.to_string())?;
            let channel_rows = channels_stmt.query_map(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            }).map_err(|e| e.to_string())?;

            let mut channels = Vec::new();
            for channel_row in channel_rows {
                let (chan_id, chan_name, chan_desc) = channel_row.map_err(|e| e.to_string())?;
                let channel_id = Uuid::parse_str(&chan_id).map_err(|e| e.to_string())?;
                
                // Get channel userlist
                let mut cu_stmt = conn.prepare("SELECT user_id FROM channel_users WHERE channel_id = ?1")
                    .map_err(|e| e.to_string())?;
                let channel_userlist: Vec<Uuid> = cu_stmt.query_map(params![chan_id], |row| {
                    let user_id_str: String = row.get(0)?;
                    Ok(Uuid::parse_str(&user_id_str).unwrap())
                }).map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;

                // Get permissions (simplified)
                let mut perm_stmt = conn.prepare(
                    "SELECT user_id, can_read, can_write FROM channel_permissions WHERE channel_id = ?1"
                ).map_err(|e| e.to_string())?;
                let mut can_read = Vec::new();
                let mut can_write = Vec::new();
                
                let perm_rows = perm_stmt.query_map(params![chan_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i32>(1)?,
                        row.get::<_, i32>(2)?,
                    ))
                }).map_err(|e| e.to_string())?;

                for perm_row in perm_rows {
                    let (uid, read, write) = perm_row.map_err(|e| e.to_string())?;
                    let uuid = Uuid::parse_str(&uid).map_err(|e| e.to_string())?;
                    if read != 0 { can_read.push(uuid); }
                    if write != 0 { can_write.push(uuid); }
                }

                channels.push(nexus_tui_common::Channel {
                    id: channel_id,
                    server_id,
                    name: chan_name,
                    description: chan_desc,
                    permissions: nexus_tui_common::ChannelPermissions { can_read, can_write },
                    userlist: channel_userlist,
                    messages: Vec::new(), // Always empty in server list
                });
            }

            servers.push(Server {
                id: server_id,
                name,
                description,
                public: public != 0,
                invite_code,
                icon,
                banner,
                owner: Uuid::parse_str(&owner).map_err(|e| e.to_string())?,
                mods,
                userlist,
                channels,
            });
        }

        Ok(servers)
    })
    .await
    .unwrap()
}

pub async fn get_default_server_id() -> Result<Option<Uuid>, String> {
    task::spawn_blocking(|| {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare("SELECT id FROM servers ORDER BY rowid ASC LIMIT 1")
            .map_err(|e| e.to_string())?;
        
        match stmt.query_row([], |row| row.get::<_, String>(0)) {
            Ok(id_str) => Ok(Some(Uuid::parse_str(&id_str).map_err(|e| e.to_string())?)),
            Err(_) => Ok(None),
        }
    })
    .await
    .unwrap()
}

/// Get all servers (simplified for user registration)
pub async fn db_get_servers() -> Result<Vec<nexus_tui_common::Server>, String> {
    task::spawn_blocking(|| {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare(
            "SELECT id, name, description, owner FROM servers ORDER BY id LIMIT 1"
        ).map_err(|e| e.to_string())?;
        
        let rows = stmt.query_map([], |row| {
            let owner_str: String = row.get(3)?;
            let owner_uuid = Uuid::parse_str(&owner_str).map_err(|_| rusqlite::Error::InvalidColumnType(3, "owner".to_string(), rusqlite::types::Type::Text))?;
            
            Ok(nexus_tui_common::Server {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|_| rusqlite::Error::InvalidColumnType(0, "id".to_string(), rusqlite::types::Type::Text))?,
                name: row.get(1)?,
                description: row.get(2)?,
                public: true,
                invite_code: None,
                icon: None,
                banner: None,
                owner: owner_uuid,
                mods: Vec::new(),
                userlist: Vec::new(),
                channels: Vec::new(),
            })
        }).map_err(|e| e.to_string())?;
        
        let mut servers = Vec::new();
        for row in rows {
            servers.push(row.map_err(|e| e.to_string())?);
        }
        
        Ok(servers)
    })
    .await
    .unwrap()
}

/// Add user to a server
pub async fn db_add_user_to_server(server_id: Uuid, user_id: Uuid) -> Result<(), String> {
    let server_id_str = server_id.to_string();
    let user_id_str = user_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        conn.execute(
            "INSERT OR IGNORE INTO server_users (server_id, user_id) VALUES (?1, ?2)",
            params![server_id_str, user_id_str],
        ).map_err(|e| e.to_string())?;
        
        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_is_user_in_server(user_id: Uuid, server_id: Uuid) -> Result<bool, String> {
    let user_id_str = user_id.to_string();
    let server_id_str = server_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM server_users WHERE user_id = ?1 AND server_id = ?2")
            .map_err(|e| e.to_string())?;
        
        let count: i64 = stmt.query_row(params![user_id_str, server_id_str], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        
        Ok(count > 0)
    })
    .await
    .unwrap()
}

pub async fn ensure_default_server_exists() -> Result<(), String> {
    task::spawn_blocking(|| {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        // Check if any servers exist
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM servers", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        
        if count > 0 {
            return Ok(());
        }

        // Get the first admin user as owner
        let mut stmt = conn.prepare("SELECT id FROM users WHERE role = 'Admin' LIMIT 1")
            .map_err(|e| e.to_string())?;
        let owner_id: String = stmt.query_row([], |row| row.get(0))
            .map_err(|_| "No admin user found".to_string())?;
        let _owner_uuid = Uuid::parse_str(&owner_id).map_err(|e| e.to_string())?;

        // Create default server
        let server_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO servers (id, name, description, public, owner) VALUES (?1, ?2, ?3, 1, ?4)",
            params![server_id.to_string(), "Nexus", "The default community server.", owner_id],
        ).map_err(|e| e.to_string())?;

        // Add owner to server_users and server_mods
        conn.execute(
            "INSERT INTO server_users (server_id, user_id) VALUES (?1, ?2)",
            params![server_id.to_string(), owner_id],
        ).map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO server_mods (server_id, user_id) VALUES (?1, ?2)",
            params![server_id.to_string(), owner_id],
        ).map_err(|e| e.to_string())?;

        // Create default channels
        let channels = [
            ("general", "General discussion"),
            ("cyberdeck", "Tech talk"),
            ("random", "Off-topic"),
        ];

        for (name, desc) in channels {
            let channel_id = Uuid::new_v4();
            conn.execute(
                "INSERT INTO channels (id, server_id, name, description) VALUES (?1, ?2, ?3, ?4)",
                params![channel_id.to_string(), server_id.to_string(), name, desc],
            ).map_err(|e| e.to_string())?;

            // Add owner to channel
            conn.execute(
                "INSERT INTO channel_users (channel_id, user_id) VALUES (?1, ?2)",
                params![channel_id.to_string(), owner_id],
            ).map_err(|e| e.to_string())?;
        }

        Ok(())
    })
    .await
    .unwrap()
}
