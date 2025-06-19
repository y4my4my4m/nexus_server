// server/src/db/migrations.rs
use rusqlite::{Connection, Result as SqlResult};

const DB_PATH: &str = "cyberpunk_bbs.db";

pub fn init_db() -> SqlResult<Connection> {
    let conn = Connection::open(DB_PATH)?;
    // Users
    conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            color TEXT NOT NULL,
            role TEXT NOT NULL
            )",
        [],
    )?;
    // --- MIGRATION: Add missing profile columns if not present ---
    let columns = [
        ("bio", "TEXT"),
        ("url1", "TEXT"),
        ("url2", "TEXT"),
        ("url3", "TEXT"),
        ("location", "TEXT"),
        ("profile_pic", "TEXT"),
        ("cover_banner", "TEXT"),
    ];
    for (col, ty) in columns.iter() {
        let sql = format!("ALTER TABLE users ADD COLUMN {} {}", col, ty);
        let res = conn.execute(&sql, []);
        if let Err(e) = res {
            if !e.to_string().contains("duplicate column name") {
                return Err(e);
            }
        }
    }
    // Forums
    conn.execute(
        "CREATE TABLE IF NOT EXISTS forums (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL
        )",
        [],
    )?;
    // Threads
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
    // Posts
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
    // Direct Messages
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
    // --- SERVERS/CHANNELS/CHANNEL_MESSAGES ---
    conn.execute(
        "CREATE TABLE IF NOT EXISTS servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            public INTEGER NOT NULL,
            invite_code TEXT,
            icon TEXT,
            banner TEXT,
            owner TEXT NOT NULL,
            FOREIGN KEY(owner) REFERENCES users(id)
        )",
        [],
    )?;
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
    conn.execute(
        "CREATE TABLE IF NOT EXISTS channel_permissions (
            channel_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            can_read INTEGER NOT NULL,
            can_write INTEGER NOT NULL,
            PRIMARY KEY(channel_id, user_id),
            FOREIGN KEY(channel_id) REFERENCES channels(id),
            FOREIGN KEY(user_id) REFERENCES users(id)
        )",
        [],
    )?;
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
    // Notifications
    conn.execute(
        "CREATE TABLE IF NOT EXISTS notifications (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            type TEXT NOT NULL,
            related_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            read INTEGER NOT NULL DEFAULT 0,
            extra TEXT
        )",
        [],
    )?;
    Ok(conn)
}
