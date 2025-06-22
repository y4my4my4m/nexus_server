use crate::util::{parse_color, parse_user_color};
use common::{Forum, Thread, Post, User, UserRole, UserStatus};
use rusqlite::{params, Connection};
use tokio::task;
use uuid::Uuid;

const DB_PATH: &str = "cyberpunk_bbs.db";

pub async fn db_get_forums() -> Result<Vec<Forum>, String> {
    task::spawn_blocking(|| {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let mut forums = Vec::new();

        let mut stmt = conn.prepare("SELECT id, name, description FROM forums")
            .map_err(|e| e.to_string())?;
        let forum_rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }).map_err(|e| e.to_string())?;

        for forum_row in forum_rows {
            let (forum_id, name, description) = forum_row.map_err(|e| e.to_string())?;
            let forum_uuid = Uuid::parse_str(&forum_id).map_err(|e| e.to_string())?;

            // Get threads for this forum
            let mut thread_stmt = conn.prepare(
                "SELECT id, title, author_id, timestamp FROM threads WHERE forum_id = ?1"
            ).map_err(|e| e.to_string())?;
            let thread_rows = thread_stmt.query_map(params![forum_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            }).map_err(|e| e.to_string())?;

            let mut threads = Vec::new();
            for thread_row in thread_rows {
                let (thread_id, title, author_id, thread_timestamp) = thread_row.map_err(|e| e.to_string())?;
                let thread_uuid = Uuid::parse_str(&thread_id).map_err(|e| e.to_string())?;

                // Get thread author
                let mut user_stmt = conn.prepare(
                    "SELECT id, username, color, role, profile_pic, cover_banner FROM users WHERE id = ?1"
                ).map_err(|e| e.to_string())?;
                let (user_id, username, color, role, profile_pic, cover_banner) = user_stmt.query_row(
                    params![author_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    }
                ).map_err(|e| e.to_string())?;

                let author = User {
                    id: Uuid::parse_str(&user_id).unwrap(),
                    username,
                    color: parse_user_color(&color),
                    role: match role.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
                    profile_pic,
                    cover_banner,
                    status: UserStatus::Offline,
                };

                // Get posts for this thread
                let mut post_stmt = conn.prepare(
                    "SELECT id, author_id, content, timestamp FROM posts WHERE thread_id = ?1"
                ).map_err(|e| e.to_string())?;
                let post_rows = post_stmt.query_map(params![thread_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                }).map_err(|e| e.to_string())?;

                let mut posts = Vec::new();
                for post_row in post_rows {
                    let (post_id, post_author_id, content, post_timestamp) = post_row.map_err(|e| e.to_string())?;

                    // Get post author
                    let mut post_user_stmt = conn.prepare(
                        "SELECT id, username, color, role, profile_pic, cover_banner FROM users WHERE id = ?1"
                    ).map_err(|e| e.to_string())?;
                    let (puser_id, pusername, pcolor, prole, pprofile_pic, pcover_banner) = post_user_stmt.query_row(
                        params![post_author_id], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, Option<String>>(4)?,
                                row.get::<_, Option<String>>(5)?,
                            ))
                        }
                    ).map_err(|e| e.to_string())?;

                    let post_author = User {
                        id: Uuid::parse_str(&puser_id).unwrap(),
                        username: pusername,
                        color: parse_user_color(&pcolor),
                        role: match prole.as_str() {
                            "Admin" => UserRole::Admin,
                            "Moderator" => UserRole::Moderator,
                            _ => UserRole::User,
                        },
                        profile_pic: pprofile_pic,
                        cover_banner: pcover_banner,
                        status: UserStatus::Offline,
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
    })
    .await
    .unwrap()
}

pub async fn db_create_thread(
    forum_id: Uuid,
    title: &str,
    author_id: Uuid,
    content: &str,
) -> Result<(), String> {
    let forum_id_str = forum_id.to_string();
    let title = title.to_string();
    let author_id_str = author_id.to_string();
    let content = content.to_string();
    let now = chrono::Utc::now().timestamp();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let thread_id = Uuid::new_v4();
        let post_id = Uuid::new_v4();

        // Insert thread
        conn.execute(
            "INSERT INTO threads (id, forum_id, title, author_id, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![thread_id.to_string(), forum_id_str, title, author_id_str, now],
        ).map_err(|e| e.to_string())?;

        // Insert first post
        conn.execute(
            "INSERT INTO posts (id, thread_id, author_id, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![post_id.to_string(), thread_id.to_string(), author_id_str, content, now],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_create_post(thread_id: Uuid, author_id: Uuid, content: &str) -> Result<(), String> {
    let thread_id_str = thread_id.to_string();
    let author_id_str = author_id.to_string();
    let content = content.to_string();
    let now = chrono::Utc::now().timestamp();

    task::spawn_blocking(move || {
        let conn = Connection::open(DB_PATH).map_err(|e| e.to_string())?;
        let post_id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO posts (id, thread_id, author_id, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![post_id.to_string(), thread_id_str, author_id_str, content, now],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}
