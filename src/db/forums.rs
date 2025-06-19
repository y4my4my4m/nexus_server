// server/src/db/forums.rs
// Forum, thread, and post DB functions

use crate::db;
use crate::util::parse_color;
use crate::db::users::db_get_user_profile;
use common::{Forum, Thread, Post, User, UserRole, UserStatus};
use uuid::Uuid;
use rusqlite::{params, Connection};

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_get_forums() -> Result<Vec<Forum>, String> {
    let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
    let mut forums = Vec::new();
    let mut stmt = conn.prepare("SELECT id, name, description FROM forums").map_err(|e| e.to_string())?;
    let forum_rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    }).map_err(|e| e.to_string())?;
    for forum_row in forum_rows {
        let (forum_id, name, description) = forum_row.map_err(|e| e.to_string())?;
        let forum_uuid = Uuid::parse_str(&forum_id).map_err(|e| e.to_string())?;
        // Get threads for this forum
        let mut thread_stmt = conn.prepare("SELECT id, title, author_id, timestamp FROM threads WHERE forum_id = ?1").map_err(|e| e.to_string())?;
        let thread_rows = thread_stmt.query_map(params![forum_id.clone()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
        }).map_err(|e| e.to_string())?;
        let mut threads = Vec::new();
        for thread_row in thread_rows {
            let (thread_id, title, author_id, thread_timestamp) = thread_row.map_err(|e| e.to_string())?;
            let thread_uuid = Uuid::parse_str(&thread_id).map_err(|e| e.to_string())?;
            // Get author user
            let mut user_stmt = conn.prepare("SELECT id, username, color, role FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
            let user_row = user_stmt.query_row(params![author_id.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            }).map_err(|e| e.to_string())?;
            let (user_id, username, color, role) = user_row;
            // Fetch author profile for profile_pic and cover_banner
            let author_profile = db_get_user_profile(Uuid::parse_str(&user_id).unwrap()).await.map_err(|e| e.to_string())?;
            let author = User {
                id: Uuid::parse_str(&user_id).unwrap(),
                username,
                color: parse_color(&color),
                role: match role.as_str() {
                    "Admin" => UserRole::Admin,
                    "Moderator" => UserRole::Moderator,
                    _ => UserRole::User,
                },
                profile_pic: author_profile.profile_pic.clone(),
                cover_banner: author_profile.cover_banner.clone(),
                status: UserStatus::Connected,
            };
            // Get posts for this thread
            let mut post_stmt = conn.prepare("SELECT id, author_id, content, timestamp FROM posts WHERE thread_id = ?1").map_err(|e| e.to_string())?;
            let post_rows = post_stmt.query_map(params![thread_id.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
            }).map_err(|e| e.to_string())?;
            let mut posts = Vec::new();
            for post_row in post_rows {
                let (post_id, post_author_id, content, post_timestamp) = post_row.map_err(|e| e.to_string())?;
                // Get post author
                let mut post_user_stmt = conn.prepare("SELECT id, username, color, role FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
                let post_user_row = post_user_stmt.query_row(params![post_author_id.clone()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
                }).map_err(|e| e.to_string())?;
                let (puser_id, pusername, pcolor, prole) = post_user_row;
                let post_author_profile = db_get_user_profile(Uuid::parse_str(&puser_id).unwrap()).await.map_err(|e| e.to_string())?;
                let post_author = User {
                    id: Uuid::parse_str(&puser_id).unwrap(),
                    username: pusername,
                    color: parse_color(&pcolor),
                    role: match prole.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
                    profile_pic: post_author_profile.profile_pic.clone(),
                    cover_banner: post_author_profile.cover_banner.clone(),
                    status: UserStatus::Connected,
                };
                posts.push(Post {
                    id: Uuid::parse_str(&post_id).unwrap(),
                    author: post_author,
                    content,
                    timestamp: post_timestamp,
                });
            }
            threads.push(Thread {
                id: thread_uuid,
                title,
                author,
                posts,
                timestamp: thread_timestamp,
            });
        }
        forums.push(Forum {
            id: forum_uuid,
            name,
            description,
            threads,
        });
    }
    Ok(forums)
}

pub async fn db_create_thread(forum_id: Uuid, title: &str, author_id: Uuid, content: &str) -> Result<(), String> {
    let forum_id = forum_id.to_string();
    let title = title.to_string();
    let author_id = author_id.to_string();
    let content = content.to_string();
    let now = chrono::Utc::now().timestamp();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let thread_id = Uuid::new_v4().to_string();
        let post_id = Uuid::new_v4().to_string();
        // Insert thread
        conn.execute(
            "INSERT INTO threads (id, forum_id, title, author_id, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![thread_id, forum_id, title, author_id, now],
        ).map_err(|e| e.to_string())?;
        // Insert first post
        conn.execute(
            "INSERT INTO posts (id, thread_id, author_id, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![post_id, thread_id, author_id, content, now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}

pub async fn db_create_post(thread_id: Uuid, author_id: Uuid, content: &str) -> Result<(), String> {
    let thread_id = thread_id.to_string();
    let author_id = author_id.to_string();
    let content = content.to_string();
    let now = chrono::Utc::now().timestamp();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let post_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO posts (id, thread_id, author_id, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![post_id, thread_id, author_id, content, now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }).await.unwrap()
}
