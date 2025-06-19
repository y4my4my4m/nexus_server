// server/src/db/users.rs
// User DB functions (registration, login, profile, etc)

use uuid::Uuid;
use crate::util::parse_color;
use crate::auth::{hash_password, verify_password};
use common::{UserProfile, UserRole};
use rusqlite::{params, Connection};
use tokio::task;

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_get_user_by_id(user_id: Uuid) -> Result<UserProfile, String> {
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT id, username, color, role, bio, url1, url2, url3, location, profile_pic, cover_banner FROM users WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let user = stmt.query_map(params![user_id], |row| {
        Ok(UserProfile {
            id: row.get(0)?,
            username: row.get(1)?,
            color: row.get(2)?,
            role: row.get(3)?,
            bio: row.get(4)?,
            url1: row.get(5)?,
            url2: row.get(6)?,
            url3: row.get(7)?,
            location: row.get(8)?,
            profile_pic: row.get(9)?,
            cover_banner: row.get(10)?,
        })
    }).map_err(|e| e.to_string())?.next().transpose().map_err(|e| e.to_string())?;
    
    user.ok_or_else(|| "User not found".to_string())
}

pub async fn db_register_user(username: &str, password: &str, color: &str, role: &str) -> Result<UserProfile, String> {
    let hashed_password = hash_password(password).map_err(|e| e.to_string())?;
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO users (username, password, color, role) VALUES (?1, ?2, ?3, ?4)",
        params![username, hashed_password, color, role],
    ).map_err(|e| e.to_string())?;
    
    let user_id = conn.last_insert_rowid();
    db_get_user_by_id(Uuid::from_u64(user_id as u64).unwrap()).await
}

pub async fn db_login_user(username: &str, password: &str) -> Result<UserProfile, String> {
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT id, password, color, role FROM users WHERE username = ?1")
        .map_err(|e| e.to_string())?;
    let user = stmt.query_map(params![username], |row| {
        Ok(UserProfile {
            id: row.get(0)?,
            username: username.to_string(),
            color: row.get(2)?,
            role: row.get(3)?,
            ..Default::default()
        })
    }).map_err(|e| e.to_string())?.next().transpose().map_err(|e| e.to_string())?;
    
    let user = user.ok_or_else(|| "User not found".to_string())?;
    verify_password(password, &user.password).map_err(|e| e.to_string())?;
    
    Ok(user)
}

pub async fn db_update_user_password(user_id: Uuid, new_password: &str) -> Result<(), String> {
    let hashed_password = hash_password(new_password).map_err(|e| e.to_string())?;
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE users SET password = ?1 WHERE id = ?2",
        params![hashed_password, user_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_update_user_color(user_id: Uuid, color: &str) -> Result<(), String> {
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE users SET color = ?1 WHERE id = ?2",
        params![color, user_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn validate_profile_fields(
    bio: &Option<String>,
    url1: &Option<String>,
    url2: &Option<String>,
    url3: &Option<String>,
    location: &Option<String>,
    profile_pic: &Option<String>,
    cover_banner: &Option<String>,
) -> Result<(), String> {
    if let Some(bio) = bio {
        if bio.len() > 5000 {
            return Err("Bio must be at most 5000 characters.".to_string());
        }
    }
    for (i, url) in [url1, url2, url3].iter().enumerate() {
        if let Some(u) = url {
            if u.len() > 100 {
                return Err(format!("URL{} must be at most 100 characters.", i + 1));
            }
        }
    }
    if let Some(loc) = location {
        if loc.len() > 100 {
            return Err("Location must be at most 100 characters.".to_string());
        }
    }
    if let Some(pic) = profile_pic {
        if pic.len() > 1024 * 1024 {
            return Err("Profile picture must be at most 1MB (base64 or URL).".to_string());
        }
    }
    if let Some(banner) = cover_banner {
        if banner.len() > 1024 * 1024 {
            return Err("Cover banner must be at most 1MB (base64 or URL).".to_string());
        }
    }
    Ok(())
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
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE users SET bio = ?1, url1 = ?2, url2 = ?3, url3 = ?4, location = ?5, profile_pic = ?6, cover_banner = ?7 WHERE id = ?8",
        params![bio, url1, url2, url3, location, profile_pic, cover_banner, user_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_get_user_profile(user_id: Uuid) -> Result<UserProfile, String> {
    db_get_user_by_id(user_id).await
}
