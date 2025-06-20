// server/src/db/users.rs

use crate::auth::{hash_password, verify_password};
use crate::util::parse_color;
use common::{UserProfile, UserRole};
use rusqlite::{params, Connection, Result as SqlResult};
use tokio::task;
use tracing::info;
use uuid::Uuid;

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_count_users() -> Result<i64, String> {
    task::spawn_blocking(|| {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count)
    })
    .await
    .unwrap()
}

pub async fn db_register_user(
    username: &str,
    password: &str,
    color: &str,
    role: &str,
) -> Result<UserProfile, String> {
    let username = username.to_string();
    let username_lower = username.to_lowercase();
    let password = password.to_string();
    let color = color.to_string();
    let role = role.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        // Check if username exists (case insensitive)
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM users WHERE LOWER(username) = ?1")
            .map_err(|e| e.to_string())?;
        let exists: i64 = stmt
            .query_row(params![username_lower], |row| row.get(0))
            .map_err(|e| e.to_string())?;

        if exists > 0 {
            return Err("Username already taken".to_string());
        }

        let id = Uuid::new_v4();
        let hash = hash_password(&password).map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO users (id, username, password_hash, color, role) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), username, hash, color, role],
        )
        .map_err(|e| e.to_string())?;

        info!("User registered: {} ({})", username, id);

        Ok(UserProfile {
            id,
            username,
            hash: String::new(), // Don't return password hash
            color: parse_color(&color),
            role: match role.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: None,
            url1: None,
            url2: None,
            url3: None,
            location: None,
            profile_pic: None,
            cover_banner: None,
        })
    })
    .await
    .unwrap()
}

pub async fn db_login_user(username: &str, password: &str) -> Result<UserProfile, String> {
    let username_lower = username.to_lowercase();
    let password = password.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare("SELECT id, username, password_hash, color, role FROM users WHERE LOWER(username) = ?1")
            .map_err(|e| e.to_string())?;

        let user = stmt
            .query_row(params![username_lower], |row| {
                let id: String = row.get(0)?;
                let username: String = row.get(1)?;
                let hash: String = row.get(2)?;
                let color: String = row.get(3)?;
                let role: String = row.get(4)?;
                Ok((id, username, hash, color, role))
            })
            .map_err(|_| "Invalid credentials".to_string())?;

        if !verify_password(&user.2, &password) {
            return Err("Invalid credentials".to_string());
        }

        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: String::new(), // Don't return password hash
            color: parse_color(&user.3),
            role: match user.4.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: None,
            url1: None,
            url2: None,
            url3: None,
            location: None,
            profile_pic: None,
            cover_banner: None,
        })
    })
    .await
    .unwrap()
}

pub async fn db_get_user_by_id(user_id: Uuid) -> Result<UserProfile, String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, color, role, bio, url1, url2, url3, location, profile_pic, cover_banner 
             FROM users WHERE id = ?1"
        ).map_err(|e| e.to_string())?;

        let user = stmt
            .query_row(params![user_id_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            })
            .map_err(|_| "User not found".to_string())?;

        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: String::new(), // Don't return password hash
            color: parse_color(&user.3),
            role: match user.4.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: user.5,
            url1: user.6,
            url2: user.7,
            url3: user.8,
            location: user.9,
            profile_pic: user.10,
            cover_banner: user.11,
        })
    })
    .await
    .unwrap()
}

pub async fn db_get_user_by_username(username: &str) -> Result<UserProfile, String> {
    let username_lower = username.to_lowercase();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT id, username, password_hash, color, role, bio, url1, url2, url3, location, profile_pic, cover_banner 
             FROM users WHERE LOWER(username) = ?1"
        ).map_err(|e| e.to_string())?;

        let user = stmt
            .query_row(params![username_lower], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            })
            .map_err(|_| "User not found".to_string())?;

        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: String::new(), // Don't return password hash
            color: parse_color(&user.3),
            role: match user.4.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: user.5,
            url1: user.6,
            url2: user.7,
            url3: user.8,
            location: user.9,
            profile_pic: user.10,
            cover_banner: user.11,
        })
    })
    .await
    .unwrap()
}

pub async fn db_update_user_password(user_id: Uuid, new_password: &str) -> Result<(), String> {
    let user_id_str = user_id.to_string();
    let new_password = new_password.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let hash = hash_password(&new_password).map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![hash, user_id_str],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_update_user_color(user_id: Uuid, color: &str) -> Result<(), String> {
    let user_id_str = user_id.to_string();
    let color = color.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE users SET color = ?1 WHERE id = ?2",
            params![color, user_id_str],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_update_user_profile(
    user_id: Uuid,
    bio: Option<String>,
    url1: Option<String>,
    url2: Option<String>,
    url3: Option<String>,
    location: Option<String>,
    profile_pic: Option<String>,
    cover_banner: Option<String>,
) -> Result<(), String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE users SET bio = ?1, url1 = ?2, url2 = ?3, url3 = ?4, location = ?5, profile_pic = ?6, cover_banner = ?7 WHERE id = ?8",
            params![bio, url1, url2, url3, location, profile_pic, cover_banner, user_id_str],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_get_user_profile(user_id: Uuid) -> Result<UserProfile, String> {
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT id, username, bio, url1, url2, url3, location, profile_pic, cover_banner, color, role 
             FROM users WHERE id = ?1"
        ).map_err(|e| e.to_string())?;

        let user = stmt
            .query_row(params![user_id_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })
            .map_err(|_| "User not found".to_string())?;

        Ok(UserProfile {
            id: Uuid::parse_str(&user.0).unwrap(),
            username: user.1,
            hash: String::new(), // Don't return password hash
            color: parse_color(&user.9),
            role: match user.10.as_str() {
                "Admin" => UserRole::Admin,
                "Moderator" => UserRole::Moderator,
                _ => UserRole::User,
            },
            bio: user.2,
            url1: user.3,
            url2: user.4,
            url3: user.5,
            location: user.6,
            profile_pic: user.7,
            cover_banner: user.8,
        })
    })
    .await
    .unwrap()
}
