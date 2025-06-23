use once_cell::sync::OnceCell;
use std::sync::RwLock;

/// Global database path configuration
static DB_CONFIG: OnceCell<RwLock<String>> = OnceCell::new();

/// Initialize the global database path
pub fn init_db_path(path: String) {
    DB_CONFIG.set(RwLock::new(path)).ok();
}

/// Get the current database path
pub fn get_db_path() -> String {
    DB_CONFIG
        .get()
        .map(|config| config.read().unwrap().clone())
        .unwrap_or_else(|| "nexus.db".to_string()) // fallback to default
}

/// Update the database path at runtime (if needed)
pub fn set_db_path(path: String) {
    if let Some(config) = DB_CONFIG.get() {
        *config.write().unwrap() = path;
    } else {
        init_db_path(path);
    }
}