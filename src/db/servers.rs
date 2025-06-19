// server/src/db/servers.rs
// Server (guild) DB functions

use common::{Server, Channel};
use rusqlite::{params, Connection};
use uuid::Uuid;

const DB_PATH: &str = "cyberpunk_bbs.db";

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
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
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
    let user_id = user_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT s.id, s.name, s.description, s.public, s.invite_code, s.icon, s.banner, s.owner FROM servers s INNER JOIN server_users su ON s.id = su.server_id WHERE su.user_id = ?1").map_err(|e| e.to_string())?;
        let server_rows = stmt.query_map(params![user_id.clone()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i32>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, String>(7)?))
        }).map_err(|e| e.to_string())?;
        let mut servers = Vec::new();
        for server_row in server_rows {
            let (id, name, description, public, invite_code, icon, banner, owner) = server_row.map_err(|e| e.to_string())?;
            let server_id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
            // Fetch mods
            let mut mods_stmt = conn.prepare("SELECT user_id FROM server_mods WHERE server_id = ?1 ORDER BY user_id ASC").map_err(|e| e.to_string())?;
            let mods = mods_stmt.query_map(params![id.clone()], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?
                .map(|r| Uuid::parse_str(&r.unwrap()).unwrap()).collect();
            // Fetch userlist
            let mut users_stmt = conn.prepare("SELECT user_id FROM server_users WHERE server_id = ?1 ORDER BY user_id ASC").map_err(|e| e.to_string())?;
            let userlist = users_stmt.query_map(params![id.clone()], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?
                .map(|r| Uuid::parse_str(&r.unwrap()).unwrap()).collect();
            // Fetch only channels the user is a member of
            let mut channels_stmt = conn.prepare("SELECT id, name, description FROM channels WHERE server_id = ?1 AND id IN (SELECT channel_id FROM channel_users WHERE user_id = ?2) ORDER BY name ASC").map_err(|e| e.to_string())?;
            let channel_rows = channels_stmt.query_map(params![id.clone(), user_id.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            }).map_err(|e| e.to_string())?;
            let mut channels = Vec::new();
            for channel_row in channel_rows {
                let (chan_id, chan_name, chan_desc) = channel_row.map_err(|e| e.to_string())?;
                let channel_id = Uuid::parse_str(&chan_id).map_err(|e| e.to_string())?;
                // Fetch channel userlist
                let mut cu_stmt = conn.prepare(
                    "SELECT cu.user_id FROM channel_users cu
                     INNER JOIN users u ON cu.user_id = u.id
                     WHERE cu.channel_id = ?1
                     ORDER BY u.username ASC"
                ).map_err(|e| e.to_string())?;
                let channel_userlist = cu_stmt.query_map(params![chan_id.clone()], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?
                    .map(|r| Uuid::parse_str(&r.unwrap()).unwrap()).collect();
                // Fetch permissions
                let mut perm_stmt = conn.prepare("SELECT user_id, can_read, can_write FROM channel_permissions WHERE channel_id = ?1").map_err(|e| e.to_string())?;
                let mut can_read = Vec::new();
                let mut can_write = Vec::new();
                let perm_rows = perm_stmt.query_map(params![chan_id.clone()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?, row.get::<_, i32>(2)?))
                }).map_err(|e| e.to_string())?;
                for perm_row in perm_rows {
                    let (uid, read, write) = perm_row.map_err(|e| e.to_string())?;
                    let uuid = Uuid::parse_str(&uid).map_err(|e| e.to_string())?;
                    if read != 0 { can_read.push(uuid); }
                    if write != 0 { can_write.push(uuid); }
                }
                channels.push(Channel {
                    id: channel_id,
                    server_id: server_id,
                    name: chan_name,
                    description: chan_desc,
                    permissions: common::ChannelPermissions { can_read, can_write },
                    userlist: channel_userlist,
                    messages: Vec::new(),
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
    }).await.unwrap()
}

pub async fn ensure_default_server_exists() -> Result<(), String> {
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM servers", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if count > 0 {
        return Ok(());
    }
    let mut stmt = conn.prepare("SELECT id FROM users WHERE role = 'Admin' LIMIT 1").map_err(|e| e.to_string())?;
    let owner_id: String = stmt.query_row([], |row| row.get(0)).map_err(|_| "No admin user found".to_string())?;
    let owner_uuid = Uuid::parse_str(&owner_id).map_err(|e| e.to_string())?;
    let server_id = db_create_server(
        "Nexus",
        "The default community server.",
        true,
        owner_uuid,
        None,
        None,
    ).await?;
    // Create default channels
    let _ = crate::db::channels::db_create_channel(server_id, "general", "General discussion").await?;
    let _ = crate::db::channels::db_create_channel(server_id, "cyberdeck", "Tech talk").await?;
    let _ = crate::db::channels::db_create_channel(server_id, "random", "Off-topic").await?;
    Ok(())
}
