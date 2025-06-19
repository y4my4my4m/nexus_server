// server/src/auth.rs
// Password hashing/verification and authentication helpers

use argon2::{self, Argon2, password_hash::{PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng, PasswordHash}};

pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2.hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    let parsed_hash = PasswordHash::new(hash);
    if let Ok(parsed_hash) = parsed_hash {
        Argon2::default().verify_password(password.as_bytes(), &parsed_hash).is_ok()
    } else {
        false
    }
}
