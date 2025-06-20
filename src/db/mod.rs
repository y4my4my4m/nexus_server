// server/src/db/mod.rs

pub mod migrations;
pub mod users;
pub mod channels;
pub mod messages;
pub mod notifications;
pub mod servers;

pub use migrations::init_db;
pub use servers::ensure_default_server_exists;
