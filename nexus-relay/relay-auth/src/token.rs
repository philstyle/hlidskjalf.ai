use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenType {
    Root,
    Admin,
    Participant,
    Invite,
}

/// Generate a secure random root key with "nrr_" prefix.
/// Format: nrr_ + 64 hex chars (32 random bytes) = 68 chars total.
pub fn generate_root_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("nrr_{}", hex::encode(bytes))
}

/// Generate a secure random admin key with "nra_" prefix.
pub fn generate_admin_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("nra_{}", hex::encode(bytes))
}

/// Generate a secure random invite key with "nri_" prefix.
pub fn generate_invite_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("nri_{}", hex::encode(bytes))
}

/// Generate a secure random participant key with "nrp_" prefix.
pub fn generate_participant_key() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("nrp_{}", hex::encode(bytes))
}

/// Hash an API key using Argon2 with a random salt.
pub fn hash_api_key(key: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let argon2 = Argon2::default();
    let hash = argon2.hash_password(key.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verify an API key against a stored Argon2 hash.
pub fn verify_api_key(key: &str, hash: &str) -> Result<bool, argon2::password_hash::Error> {
    let parsed_hash = PasswordHash::new(hash)?;
    match Argon2::default().verify_password(key.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Extract the key prefix for indexed DB lookup.
/// Returns the first 12 characters: 4-char type prefix + 8-char lookup prefix.
pub fn extract_key_prefix(key: &str) -> &str {
    let end = key.len().min(12);
    &key[..end]
}

/// Determine the token type from its prefix.
pub fn token_type(key: &str) -> Option<TokenType> {
    if key.len() < 4 {
        return None;
    }
    match &key[..4] {
        "nrr_" => Some(TokenType::Root),
        "nra_" => Some(TokenType::Admin),
        "nrp_" => Some(TokenType::Participant),
        "nri_" => Some(TokenType::Invite),
        _ => None,
    }
}

use rand::Rng as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_root_key() {
        let key = generate_root_key();
        assert!(key.starts_with("nrr_"));
        assert_eq!(key.len(), 68);
    }

    #[test]
    fn test_generate_admin_key() {
        let key = generate_admin_key();
        assert!(key.starts_with("nra_"));
        assert_eq!(key.len(), 68);
    }

    #[test]
    fn test_generate_participant_key() {
        let key = generate_participant_key();
        assert!(key.starts_with("nrp_"));
        assert_eq!(key.len(), 68);
    }

    #[test]
    fn test_extract_key_prefix() {
        let key = generate_root_key();
        let prefix = extract_key_prefix(&key);
        assert_eq!(prefix.len(), 12);
        assert!(prefix.starts_with("nrr_"));
    }

    #[test]
    fn test_token_type() {
        assert_eq!(token_type("nrr_abc"), Some(TokenType::Root));
        assert_eq!(token_type("nra_abc"), Some(TokenType::Admin));
        assert_eq!(token_type("nrp_abc"), Some(TokenType::Participant));
        assert_eq!(token_type("bad_abc"), None);
        assert_eq!(token_type("x"), None);
    }

    #[test]
    fn test_hash_and_verify() {
        let key = generate_root_key();
        let hash = hash_api_key(&key).unwrap();
        assert!(verify_api_key(&key, &hash).unwrap());
        assert!(!verify_api_key("wrong_key", &hash).unwrap());
    }
}
