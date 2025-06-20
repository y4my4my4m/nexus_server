use std::fmt;

#[derive(Debug)]
pub enum ServerError {
    Database(String),
    Authentication(String),
    Authorization(String),
    Validation(String),
    Network(String),
    Internal(String),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerError::Database(msg) => write!(f, "Database error: {}", msg),
            ServerError::Authentication(msg) => write!(f, "Authentication error: {}", msg),
            ServerError::Authorization(msg) => write!(f, "Authorization error: {}", msg),
            ServerError::Validation(msg) => write!(f, "Validation error: {}", msg),
            ServerError::Network(msg) => write!(f, "Network error: {}", msg),
            ServerError::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for ServerError {}

impl From<rusqlite::Error> for ServerError {
    fn from(err: rusqlite::Error) -> Self {
        ServerError::Database(err.to_string())
    }
}

impl From<tokio::task::JoinError> for ServerError {
    fn from(err: tokio::task::JoinError) -> Self {
        ServerError::Internal(err.to_string())
    }
}

impl From<String> for ServerError {
    fn from(err: String) -> Self {
        ServerError::Internal(err)
    }
}

pub type Result<T> = std::result::Result<T, ServerError>;