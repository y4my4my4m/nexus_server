use crate::db::db_config;
use crate::util::parse_user_color;
use nexus_tui_common::{Forum, Thread, Post, User, UserRole, UserStatus, UserInfo, ForumLightweight, ThreadLightweight, PostLightweight};
use rusqlite::{params, Connection};
use tokio::task;
use uuid::Uuid;

/// Get forums with lightweight user info (no profile images) for better performance
pub async fn db_get_forums_lightweight() -> Result<Vec<ForumLightweight>, String> {
    task::spawn_blocking(|| {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
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

                // Get thread author (lightweight - no profile images)
                let mut user_stmt = conn.prepare(
                    "SELECT id, username, color, role FROM users WHERE id = ?1"
                ).map_err(|e| e.to_string())?;
                let (user_id, username, color, role) = user_stmt.query_row(
                    params![author_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    }
                ).map_err(|e| e.to_string())?;

                let author = UserInfo {
                    id: Uuid::parse_str(&user_id).unwrap(),
                    username,
                    color: parse_user_color(&color),
                    role: match role.as_str() {
                        "Admin" => UserRole::Admin,
                        "Moderator" => UserRole::Moderator,
                        _ => UserRole::User,
                    },
                    status: UserStatus::Offline,
                };

                // Get posts for this thread
                let mut post_stmt = conn.prepare(
                    "SELECT id, author_id, content, timestamp, reply_to FROM posts WHERE thread_id = ?1"
                ).map_err(|e| e.to_string())?;
                let post_rows = post_stmt.query_map(params![thread_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                }).map_err(|e| e.to_string())?;

                let mut posts = Vec::new();
                for post_row in post_rows {
                    let (post_id, post_author_id, content, post_timestamp, reply_to_str) = post_row.map_err(|e| e.to_string())?;

                    // Parse reply_to UUID if present
                    let reply_to = match reply_to_str {
                        Some(ref s) => Uuid::parse_str(s).ok(),
                        None => None,
                    };

                    // Get post author (lightweight - no profile images)
                    let mut post_user_stmt = conn.prepare(
                        "SELECT id, username, color, role FROM users WHERE id = ?1"
                    ).map_err(|e| e.to_string())?;
                    let (puser_id, pusername, pcolor, prole) = post_user_stmt.query_row(
                        params![post_author_id], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        }
                    ).map_err(|e| e.to_string())?;

                    let post_author = UserInfo {
                        id: Uuid::parse_str(&puser_id).unwrap(),
                        username: pusername,
                        color: parse_user_color(&pcolor),
                        role: match prole.as_str() {
                            "Admin" => UserRole::Admin,
                            "Moderator" => UserRole::Moderator,
                            _ => UserRole::User,
                        },
                        status: UserStatus::Offline,
                    };

                    posts.push(PostLightweight {
                        id: Uuid::parse_str(&post_id).unwrap(),
                        author: post_author,
                        content,
                        timestamp: post_timestamp,
                        reply_to,
                    });
                }

                threads.push(ThreadLightweight {
                    id: thread_uuid,
                    title,
                    author,
                    posts,
                    timestamp: thread_timestamp,
                });
            }

            forums.push(ForumLightweight {
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

pub async fn db_get_forums() -> Result<Vec<Forum>, String> {
    task::spawn_blocking(|| {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
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
                    "SELECT id, author_id, content, timestamp, reply_to FROM posts WHERE thread_id = ?1"
                ).map_err(|e| e.to_string())?;
                let post_rows = post_stmt.query_map(params![thread_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                }).map_err(|e| e.to_string())?;

                let mut posts = Vec::new();
                for post_row in post_rows {
                    let (post_id, post_author_id, content, post_timestamp, reply_to_str) = post_row.map_err(|e| e.to_string())?;

                    // Parse reply_to UUID if present
                    let reply_to = match reply_to_str {
                        Some(ref s) => Uuid::parse_str(s).ok(),
                        None => None,
                    };

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
                        reply_to,
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
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
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

pub async fn db_create_post(thread_id: Uuid, author_id: Uuid, content: &str, reply_to: Option<Uuid>) -> Result<(), String> {
    let thread_id_str = thread_id.to_string();
    let author_id_str = author_id.to_string();
    let content = content.to_string();
    let reply_to_str = reply_to.map(|id| id.to_string());
    let now = chrono::Utc::now().timestamp();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        let post_id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO posts (id, thread_id, author_id, content, timestamp, reply_to) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![post_id.to_string(), thread_id_str, author_id_str, content, now, reply_to_str],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_create_forum(name: &str, description: &str) -> Result<(), String> {
    let name = name.to_string();
    let description = description.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        let forum_id = Uuid::new_v4();

        conn.execute(
            "INSERT INTO forums (id, name, description) VALUES (?1, ?2, ?3)",
            params![forum_id.to_string(), name, description],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_delete_post(post_id: Uuid, user_id: Uuid) -> Result<(), String> {
    let post_id_str = post_id.to_string();
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        // Check if the user owns the post or is an admin/moderator
        let mut stmt = conn.prepare(
            "SELECT author_id FROM posts WHERE id = ?1"
        ).map_err(|e| e.to_string())?;
        
        let post_author_id: String = stmt.query_row(params![post_id_str], |row| {
            row.get(0)
        }).map_err(|_| "Post not found".to_string())?;
        
        // Check user role
        let mut user_stmt = conn.prepare(
            "SELECT role FROM users WHERE id = ?1"
        ).map_err(|e| e.to_string())?;
        
        let user_role: String = user_stmt.query_row(params![user_id_str], |row| {
            row.get(0)
        }).map_err(|_| "User not found".to_string())?;
        
        // Allow deletion if user owns the post or is admin/moderator
        if post_author_id != user_id_str && user_role != "Admin" && user_role != "Moderator" {
            return Err("Permission denied: You can only delete your own posts".to_string());
        }
        
        // Delete the post
        conn.execute(
            "DELETE FROM posts WHERE id = ?1",
            params![post_id_str],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_delete_thread(thread_id: Uuid, user_id: Uuid) -> Result<(), String> {
    let thread_id_str = thread_id.to_string();
    let user_id_str = user_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        // Check if the user owns the thread or is an admin/moderator
        let mut stmt = conn.prepare(
            "SELECT author_id FROM threads WHERE id = ?1"
        ).map_err(|e| e.to_string())?;
        
        let thread_author_id: String = stmt.query_row(params![thread_id_str], |row| {
            row.get(0)
        }).map_err(|_| "Thread not found".to_string())?;
        
        // Check user role
        let mut user_stmt = conn.prepare(
            "SELECT role FROM users WHERE id = ?1"
        ).map_err(|e| e.to_string())?;
        
        let user_role: String = user_stmt.query_row(params![user_id_str], |row| {
            row.get(0)
        }).map_err(|_| "User not found".to_string())?;
        
        // Allow deletion if user owns the thread or is admin/moderator
        if thread_author_id != user_id_str && user_role != "Admin" && user_role != "Moderator" {
            return Err("Permission denied: You can only delete your own threads".to_string());
        }
        
        // Delete all posts in the thread first (foreign key constraint)
        conn.execute(
            "DELETE FROM posts WHERE thread_id = ?1",
            params![thread_id_str],
        ).map_err(|e| e.to_string())?;
        
        // Delete the thread
        conn.execute(
            "DELETE FROM threads WHERE id = ?1",
            params![thread_id_str],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_delete_forum(forum_id: Uuid) -> Result<(), String> {
    let forum_id_str = forum_id.to_string();

    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;

        // Delete all posts in threads of this forum first
        conn.execute(
            "DELETE FROM posts WHERE thread_id IN (SELECT id FROM threads WHERE forum_id = ?1)",
            params![forum_id_str],
        ).map_err(|e| e.to_string())?;

        // Delete all threads in this forum
        conn.execute(
            "DELETE FROM threads WHERE forum_id = ?1",
            params![forum_id_str],
        ).map_err(|e| e.to_string())?;

        // Delete the forum
        conn.execute(
            "DELETE FROM forums WHERE id = ?1",
            params![forum_id_str],
        ).map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
    .unwrap()
}

pub async fn db_get_post_author(post_id: Uuid) -> Result<Uuid, String> {
    let post_id_str = post_id.to_string();
    
    task::spawn_blocking(move || {
        let conn = Connection::open(db_config::get_db_path()).map_err(|e| e.to_string())?;
        
        let mut stmt = conn.prepare("SELECT author_id FROM posts WHERE id = ?1").map_err(|e| e.to_string())?;
        let author_id_str: String = stmt.query_row(params![post_id_str], |row| {
            row.get(0)
        }).map_err(|_| "Post not found".to_string())?;
        
        Uuid::parse_str(&author_id_str).map_err(|e| e.to_string())
    })
    .await
    .unwrap()
}
