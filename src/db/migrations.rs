// server/src/db/migrations.rs

use crate::errors::{Result, ServerError};
use rusqlite::{Connection, Result as SqlResult};
use tracing::info;

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn init_db() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        let conn = Connection::open(DB_PATH)?;
        create_tables(&conn)?;
        add_missing_columns(&conn)?;
        Ok::<(), rusqlite::Error>(())
    })
    .await
    .map_err(|e| ServerError::Internal(e.to_string()))?
    .map_err(|e| ServerError::Database(e.to_string()))?;

    info!("Database initialized successfully");
    Ok(())
}

fn create_tables(conn: &Connection) -> SqlResult<()> {
    // Users table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            color TEXT NOT NULL,
            role TEXT NOT NULL,
            bio TEXT,
            url1 TEXT,
            url2 TEXT,
            url3 TEXT,
            location TEXT,
            profile_pic TEXT,
            cover_banner TEXT
        )",
        [],
    )?;

    // Servers table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            public INTEGER NOT NULL DEFAULT 1,
            invite_code TEXT,
            icon TEXT,
            banner TEXT,
            owner TEXT NOT NULL,
            FOREIGN KEY(owner) REFERENCES users(id)
        )",
        [],
    )?;

    // Server users (membership)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS server_users (
            server_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            PRIMARY KEY(server_id, user_id),
            FOREIGN KEY(server_id) REFERENCES servers(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Server moderators
    conn.execute(
        "CREATE TABLE IF NOT EXISTS server_mods (
            server_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            PRIMARY KEY(server_id, user_id),
            FOREIGN KEY(server_id) REFERENCES servers(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Channels table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channels (
            id TEXT PRIMARY KEY,
            server_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            FOREIGN KEY(server_id) REFERENCES servers(id)
        )",
        [],
    )?;

    // Channel users (membership)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channel_users (
            channel_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            PRIMARY KEY(channel_id, user_id),
            FOREIGN KEY(channel_id) REFERENCES channels(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Channel permissions
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channel_permissions (
            channel_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            can_read INTEGER NOT NULL DEFAULT 1,
            can_write INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY(channel_id, user_id),
            FOREIGN KEY(channel_id) REFERENCES channels(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Channel messages
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channel_messages (
            id TEXT PRIMARY KEY,
            channel_id TEXT NOT NULL,
            sent_by TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            content TEXT NOT NULL,
            FOREIGN KEY(channel_id) REFERENCES channels(id),
            FOREIGN KEY(sent_by) REFERENCES users(id)
        )",
        [],
    )?;

    // Direct messages
    conn.execute(
        "CREATE TABLE IF NOT EXISTS direct_messages (
            id TEXT PRIMARY KEY,
            from_user_id TEXT NOT NULL,
            to_user_id TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            FOREIGN KEY(from_user_id) REFERENCES users(id),
            FOREIGN KEY(to_user_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Notifications
    conn.execute(
        "CREATE TABLE IF NOT EXISTS notifications (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            type TEXT NOT NULL,
            related_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            read INTEGER NOT NULL DEFAULT 0,
            extra TEXT,
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Forums (legacy support)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS forums (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL
        )",
        [],
    )?;

    // Threads (legacy support)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS threads (
            id TEXT PRIMARY KEY,
            forum_id TEXT NOT NULL,
            title TEXT NOT NULL,
            author_id TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            FOREIGN KEY(forum_id) REFERENCES forums(id),
            FOREIGN KEY(author_id) REFERENCES users(id)
        )",
        [],
    )?;

    // Posts (legacy support)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS posts (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            author_id TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            FOREIGN KEY(thread_id) REFERENCES threads(id),
            FOREIGN KEY(author_id) REFERENCES users(id)
        )",
        [],
    )?;

    info!("Database tables created/verified");
    Ok(())
}

fn add_missing_columns(conn: &Connection) -> SqlResult<()> {
    // Add any missing columns for backward compatibility
    let columns = [
        ("bio", "TEXT"),
        ("url1", "TEXT"),
        ("url2", "TEXT"),
        ("url3", "TEXT"),
        ("location", "TEXT"),
        ("profile_pic", "TEXT"),
        ("cover_banner", "TEXT"),
    ];

    for (col, col_type) in columns.iter() {
        let sql = format!("ALTER TABLE users ADD COLUMN {} {}", col, col_type);
        let result = conn.execute(&sql, []);
        
        if let Err(e) = result {
            // Ignore duplicate column errors
            if !e.to_string().contains("duplicate column name") {
                return Err(e);
            }
        }
    }

    // Create indexes for better performance
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_channel_messages_channel_timestamp ON channel_messages(channel_id, timestamp)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_direct_messages_users_timestamp ON direct_messages(from_user_id, to_user_id, timestamp)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_notifications_user_created ON notifications(user_id, created_at)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_server_users_server ON server_users(server_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_channel_users_channel ON channel_users(channel_id)", []);

    info!("Database migration completed");
    Ok(())
}
