//! Admin authentication primitives: argon2id password hash and HS256 JWT.

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, TokenData, Validation};
use rand::rngs::OsRng;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::{
    crypto::JwtKey,
    error::{CoreError, Result},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub iat: usize,
    /// Token generation marker. Every password rotation bumps
    /// `settings.token_generation`; tokens whose `tgen` lags the current value
    /// are rejected by the auth middleware and force a re-login.
    #[serde(default)]
    pub tgen: i64,
}

/// Hash a plaintext password with argon2id and default parameters.
pub fn hash_password(password: &SecretString) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon = Argon2::default();
    let hash = argon
        .hash_password(password.expose_secret().as_bytes(), &salt)
        .map_err(|e| CoreError::Auth(format!("hash password: {e}")))?;
    Ok(hash.to_string())
}

/// Verify `password` against a stored argon2 phc string.
pub fn verify_password(password: &SecretString, phc: &str) -> Result<bool> {
    let parsed = PasswordHash::new(phc).map_err(|e| CoreError::Auth(format!("parse phc: {e}")))?;
    match Argon2::default().verify_password(password.expose_secret().as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(CoreError::Auth(format!("verify: {e}"))),
    }
}

pub fn issue_jwt(subject: &str, tgen: i64, ttl_secs: u64, key: &JwtKey) -> Result<(String, i64)> {
    let now = Utc::now();
    let expiry = now + Duration::seconds(ttl_secs as i64);
    let claims = Claims {
        sub: subject.to_owned(),
        exp: expiry.timestamp() as usize,
        iat: now.timestamp() as usize,
        tgen,
    };
    let encoded = encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(key.as_bytes()),
    )
    .map_err(|e| CoreError::Auth(format!("jwt encode: {e}")))?;
    Ok((encoded, expiry.timestamp()))
}

pub fn decode_jwt(token: &str, key: &JwtKey) -> Result<TokenData<Claims>> {
    let mut v = Validation::new(jsonwebtoken::Algorithm::HS256);
    v.leeway = 5;
    decode::<Claims>(token, &DecodingKey::from_secret(key.as_bytes()), &v)
        .map_err(|e| CoreError::Auth(format!("jwt decode: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;

    #[test]
    fn hash_and_verify_round_trip() {
        let pw = SecretString::from("hunter2!");
        let phc = hash_password(&pw).unwrap();
        assert!(verify_password(&pw, &phc).unwrap());
        assert!(!verify_password(&SecretString::from("wrong"), &phc).unwrap());
    }

    #[test]
    fn jwt_round_trip() {
        let master = MasterKey::generate();
        let key = master.jwt_key();
        let (token, _exp) = issue_jwt("admin", 7, 60, &key).unwrap();
        let decoded = decode_jwt(&token, &key).unwrap();
        assert_eq!(decoded.claims.sub, "admin");
        assert_eq!(decoded.claims.tgen, 7);
    }
}
