// server/src/db/forums.rs
// Forum, thread, and post DB functions

use crate::util::parse_color;
use crate::db::users::db_get_user_profile;
use common::{Forum, Thread, Post, User, UserRole, UserStatus};
use uuid::Uuid;
use rusqlite::{params, Connection};

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_get_forums() -> Result<Vec<Forum>, String> {
    // Step 1: Fetch all forum/thread/post/user info in a blocking task
    let forums_data = tokio::task::spawn_blocking(|| -> Result<_, String> {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT id, name, description FROM forums").map_err(|e| e.to_string())?;
        let forum_rows: Vec<_> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        }).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        let mut forums = Vec::new();
        for (forum_id, name, description) in forum_rows {
            let forum_uuid = Uuid::parse_str(&forum_id).map_err(|e| e.to_string())?;
            let mut thread_stmt = conn.prepare("SELECT id, title, author_id, timestamp FROM threads WHERE forum_id = ?1").map_err(|e| e.to_string())?;
            let thread_rows: Vec<_> = thread_stmt.query_map(params![forum_id.clone()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
            }).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
            let mut threads = Vec::new();
            for (thread_id, title, author_id, thread_timestamp) in thread_rows {
                let thread_uuid = Uuid::parse_str(&thread_id).map_err(|e| e.to_string())?;
                let mut user_stmt = conn.prepare("SELECT id, username, color, role FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
                let user_row = user_stmt.query_row(params![author_id.clone()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
                }).map_err(|e| e.to_string())?;
                let (user_id, username, color, role) = user_row;
                let mut post_stmt = conn.prepare("SELECT id, author_id, content, timestamp FROM posts WHERE thread_id = ?1").map_err(|e| e.to_string())?;
                let post_rows: Vec<_> = post_stmt.query_map(params![thread_id.clone()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
                }).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
                let mut posts = Vec::new();
                for (post_id, post_author_id, content, post_timestamp) in post_rows {
                    let mut post_user_stmt = conn.prepare("SELECT id, username, color, role FROM users WHERE id = ?1").map_err(|e| e.to_string())?;
                    let post_user_row = post_user_stmt.query_row(params![post_author_id.clone()], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
                    }).map_err(|e| e.to_string())?;
                    let (puser_id, pusername, pcolor, prole) = post_user_row;
                    posts.push((post_id, puser_id, pusername, pcolor, prole, content, post_timestamp));
                }
                threads.push((thread_uuid, title, user_id, username, color, role, posts, thread_timestamp));
            }
            forums.push((forum_uuid, name, description, threads));
        }
        Ok(forums)
    }).await.map_err(|e| e.to_string())??;

    // Step 2: For each forum/thread/post, fetch user profiles async
    let mut forums_result = Vec::new();
    for (forum_uuid, name, description, threads) in forums_data {
        let mut threads_result = Vec::new();
        for (thread_uuid, title, user_id, username, color, role, posts, thread_timestamp) in threads {
            let author_profile = db_get_user_profile(Uuid::parse_str(&user_id).map_err(|e| e.to_string())?).await?;
            let author = User {
                id: Uuid::parse_str(&user_id).map_err(|e| e.to_string())?,
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
            let mut posts_result = Vec::new();
            for (post_id, puser_id, pusername, pcolor, prole, content, post_timestamp) in posts {
                let post_author_profile = db_get_user_profile(Uuid::parse_str(&puser_id).map_err(|e| e.to_string())?).await?;
                let post_author = User {
                    id: Uuid::parse_str(&puser_id).map_err(|e| e.to_string())?,
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
                posts_result.push(Post {
                    id: Uuid::parse_str(&post_id).map_err(|e| e.to_string())?,
                    author: post_author,
                    content,
                    timestamp: post_timestamp,
                });
            }
            threads_result.push(Thread {
                id: thread_uuid,
                title,
                author,
                posts: posts_result,
                timestamp: thread_timestamp,
            });
        }
        forums_result.push(Forum {
            id: forum_uuid,
            name,
            description,
            threads: threads_result,
        });
    }
    Ok(forums_result)
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
