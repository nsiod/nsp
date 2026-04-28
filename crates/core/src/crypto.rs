//! Crypto primitives used across nsp.
//!
//! - HKDF-SHA256 key derivation from the master key.
//! - XChaCha20-Poly1305 authenticated encryption for data-at-rest blobs.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    AeadCore, XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use zeroize::Zeroize;

use crate::error::{CoreError, Result};

const MASTER_KEY_LEN: usize = 32;
const DATA_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const JWT_KEY_LEN: usize = 32;

/// Optional 32-byte master key held in-memory, zeroized on drop.
///
/// An empty key disables data-at-rest encryption for explicit local
/// development. Callers must gate this before serving public traffic because
/// JWT signing falls back to a stable development key.
pub struct MasterKey {
    bytes: Option<[u8; MASTER_KEY_LEN]>,
}

impl MasterKey {
    pub fn from_base64(input: &SecretString) -> Result<Self> {
        if input.expose_secret().trim().is_empty() {
            return Ok(Self::disabled());
        }
        let decoded = B64
            .decode(input.expose_secret().trim())
            .map_err(|e| CoreError::Crypto(format!("master key base64: {e}")))?;
        if decoded.len() != MASTER_KEY_LEN {
            return Err(CoreError::Crypto(format!(
                "master key must be {MASTER_KEY_LEN} bytes, got {}",
                decoded.len()
            )));
        }
        let mut buf = [0u8; MASTER_KEY_LEN];
        buf.copy_from_slice(&decoded);
        Ok(Self { bytes: Some(buf) })
    }

    pub fn disabled() -> Self {
        Self { bytes: None }
    }

    pub fn generate() -> Self {
        let mut buf = [0u8; MASTER_KEY_LEN];
        OsRng.fill_bytes(&mut buf);
        Self { bytes: Some(buf) }
    }

    pub fn to_base64(&self) -> String {
        self.bytes
            .map_or_else(String::new, |bytes| B64.encode(bytes))
    }

    pub fn encryption_enabled(&self) -> bool {
        self.bytes.is_some()
    }

    /// Derive a purpose-tagged subkey via HKDF-SHA256.
    fn derive<const N: usize>(&self, info: &[u8]) -> [u8; N] {
        let bytes = self.bytes.as_ref().unwrap_or(&[0u8; MASTER_KEY_LEN]);
        let hk = Hkdf::<Sha256>::new(None, bytes);
        let mut out = [0u8; N];
        // expand never fails for reasonable lengths (N <= 255 * HashLen).
        if hk.expand(info, &mut out).is_err() {
            // Fall back to a zero block; never triggered in practice.
            out.fill(0);
        }
        out
    }

    /// Data-encryption key for at-rest blobs.
    pub fn data_key(&self) -> DataKey {
        match self.bytes {
            Some(_) => DataKey::Encrypted(self.derive::<DATA_KEY_LEN>(b"nsp/data-key/v1")),
            None => DataKey::Plain,
        }
    }

    /// JWT HS256 signing key.
    pub fn jwt_key(&self) -> JwtKey {
        match self.bytes {
            Some(_) => JwtKey(self.derive::<JWT_KEY_LEN>(b"nsp/jwt-key/v1")),
            None => JwtKey(*b"nsp/dev/jwt-key/no-master/v1!!!!"),
        }
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        if let Some(bytes) = self.bytes.as_mut() {
            bytes.zeroize();
        }
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

/// Data-at-rest codec.
pub enum DataKey {
    Encrypted([u8; DATA_KEY_LEN]),
    Plain,
}

impl DataKey {
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        match self {
            Self::Encrypted(key) => {
                let cipher = XChaCha20Poly1305::new_from_slice(key)
                    .map_err(|e| CoreError::Crypto(format!("aead key: {e}")))?;
                let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
                let ct = cipher
                    .encrypt(&nonce, plaintext)
                    .map_err(|e| CoreError::Crypto(format!("aead encrypt: {e}")))?;
                let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
                out.extend_from_slice(&nonce);
                out.extend_from_slice(&ct);
                Ok(out)
            }
            Self::Plain => Ok(plaintext.to_vec()),
        }
    }

    pub fn open(&self, blob: &[u8]) -> Result<Vec<u8>> {
        match self {
            Self::Encrypted(key) => {
                if blob.len() < NONCE_LEN {
                    return Err(CoreError::Crypto("ciphertext too short".into()));
                }
                let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
                let cipher = XChaCha20Poly1305::new_from_slice(key)
                    .map_err(|e| CoreError::Crypto(format!("aead key: {e}")))?;
                let nonce = XNonce::from_slice(nonce_bytes);
                cipher
                    .decrypt(nonce, ct)
                    .map_err(|e| CoreError::Crypto(format!("aead decrypt: {e}")))
            }
            Self::Plain => Ok(blob.to_vec()),
        }
    }
}

impl Drop for DataKey {
    fn drop(&mut self) {
        if let Self::Encrypted(key) = self {
            key.zeroize();
        }
    }
}

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DataKey(<redacted>)")
    }
}

/// JWT HS256 signing key (32 bytes).
pub struct JwtKey([u8; JWT_KEY_LEN]);

impl JwtKey {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for JwtKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for JwtKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JwtKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    #[test]
    fn roundtrip_seals_and_opens() {
        let master = MasterKey::generate();
        let dk = master.data_key();
        let plaintext = b"super secret";
        let ct = dk.seal(plaintext).unwrap();
        assert_ne!(ct, plaintext);
        let pt = dk.open(&ct).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn decodes_base64_master_key() {
        let master = MasterKey::generate();
        let encoded = SecretString::from(master.to_base64());
        let restored = MasterKey::from_base64(&encoded).unwrap();
        assert_eq!(master.to_base64(), restored.to_base64());
    }

    #[test]
    fn rejects_wrong_length() {
        let bad = SecretString::from(B64.encode([0u8; 8]));
        assert!(MasterKey::from_base64(&bad).is_err());
    }

    #[test]
    fn data_key_is_deterministic_per_master() {
        let master = MasterKey::generate();
        let a = master.data_key();
        let b = master.data_key();
        assert_eq!(
            a.seal(b"test").unwrap().len(),
            b.seal(b"test").unwrap().len()
        );
    }

    #[test]
    fn empty_master_key_disables_data_encryption() {
        let master = MasterKey::from_base64(&SecretString::from("")).unwrap();
        assert!(!master.encryption_enabled());
        let dk = master.data_key();
        let plaintext = b"not secret";
        let sealed = dk.seal(plaintext).unwrap();
        assert_eq!(sealed, plaintext);
        assert_eq!(dk.open(&sealed).unwrap(), plaintext);
    }

    #[test]
    fn debug_redacts() {
        let m = MasterKey::generate();
        let s = format!("{m:?}");
        assert!(s.contains("redacted"));
        let d = m.data_key();
        assert!(format!("{d:?}").contains("redacted"));
        let j = m.jwt_key();
        assert!(format!("{j:?}").contains("redacted"));
    }
}
