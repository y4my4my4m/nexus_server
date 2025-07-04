use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use std::error::Error;

/// Validate password meets minimum requirements
pub fn validate_password(password: &str) -> Result<(), String> {
    if password.trim().is_empty() {
        return Err("Password cannot be empty".to_string());
    }
    
    // Use minimum length of 6 characters as requested
    const MIN_PASSWORD_LENGTH: usize = 6;
    
    if password.len() < MIN_PASSWORD_LENGTH {
        return Err(format!("Password must be at least {} characters long", MIN_PASSWORD_LENGTH));
    }
    
    Ok(())
}

/// Hash a password using Argon2
pub fn hash_password(password: &str) -> Result<String, Box<dyn Error>> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string().into())
}

/// Verify a password against its hash
pub fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .and_then(|parsed_hash| {
            Argon2::default().verify_password(password.as_bytes(), &parsed_hash)
        })
        .is_ok()
}
