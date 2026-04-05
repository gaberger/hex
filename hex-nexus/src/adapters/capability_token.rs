//! Capability token service — signing and verification (ADR-2604051800 P1).
//!
//! Signs `AgentCapabilityToken` payloads with HMAC-SHA256 and verifies
//! tokens presented by agents. The signing key is derived from `HEX_TOKEN_SECRET`
//! env var, or a random 32-byte key generated at startup (single-instance mode).

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hex_core::domain::capability::{AgentCapabilityToken, Capability, VerifiedClaims};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Service for signing and verifying agent capability tokens.
#[derive(Clone)]
pub struct CapabilityTokenService {
    secret: Vec<u8>,
}

impl CapabilityTokenService {
    /// Create a new service with the given secret key.
    pub fn new(secret: Vec<u8>) -> Self {
        assert!(!secret.is_empty(), "capability token secret must not be empty");
        Self { secret }
    }

    /// Create from env var `HEX_TOKEN_SECRET`, or generate a random key.
    pub fn from_env() -> Self {
        let secret = match std::env::var("HEX_TOKEN_SECRET") {
            Ok(s) if !s.is_empty() => s.into_bytes(),
            _ => {
                let mut key = vec![0u8; 32];
                // Use system randomness
                use std::collections::hash_map::RandomState;
                use std::hash::{BuildHasher, Hasher};
                let s = RandomState::new();
                for chunk in key.chunks_mut(8) {
                    let val = s.build_hasher().finish().to_le_bytes();
                    for (dst, src) in chunk.iter_mut().zip(val.iter()) {
                        *dst = *src;
                    }
                }
                tracing::info!("Generated ephemeral capability token secret (single-instance mode)");
                key
            }
        };
        Self::new(secret)
    }

    /// Issue a signed token for an agent with the given capabilities.
    pub fn issue(
        &self,
        agent_id: &str,
        swarm_id: Option<&str>,
        project_dir: Option<&str>,
        capabilities: Vec<Capability>,
        ttl_secs: u64,
    ) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let token = AgentCapabilityToken {
            agent_id: agent_id.to_string(),
            swarm_id: swarm_id.map(String::from),
            project_dir: project_dir.map(String::from),
            capabilities,
            issued_at: now,
            expires_at: if ttl_secs > 0 { now + ttl_secs } else { 0 },
        };

        let payload = serde_json::to_vec(&token).expect("token serialization");
        let signature = self.sign(&payload);
        let payload_b64 = URL_SAFE_NO_PAD.encode(&payload);
        let sig_b64 = URL_SAFE_NO_PAD.encode(&signature);

        format!("{}.{}", payload_b64, sig_b64)
    }

    /// Verify a token string and return the claims if valid.
    pub fn verify(&self, token_str: &str) -> Result<VerifiedClaims, TokenError> {
        let (payload_b64, sig_b64) = token_str
            .rsplit_once('.')
            .ok_or(TokenError::MalformedToken)?;

        let payload = URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|_| TokenError::MalformedToken)?;
        let signature = URL_SAFE_NO_PAD
            .decode(sig_b64)
            .map_err(|_| TokenError::MalformedToken)?;

        // Verify HMAC signature
        let expected = self.sign(&payload);
        if !constant_time_eq(&signature, &expected) {
            return Err(TokenError::InvalidSignature);
        }

        let token: AgentCapabilityToken =
            serde_json::from_slice(&payload).map_err(|_| TokenError::MalformedToken)?;

        // Check expiry
        if token.expires_at > 0 {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if now > token.expires_at {
                return Err(TokenError::Expired);
            }
        }

        Ok(VerifiedClaims {
            agent_id: token.agent_id,
            swarm_id: token.swarm_id,
            project_dir: token.project_dir,
            capabilities: token.capabilities,
        })
    }

    fn sign(&self, payload: &[u8]) -> Vec<u8> {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(payload);
        mac.finalize().into_bytes().to_vec()
    }
}

/// Constant-time comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Errors that can occur during token verification.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("malformed token")]
    MalformedToken,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("token expired")]
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_sign_verify() {
        let svc = CapabilityTokenService::new(b"test-secret-key-32bytes!abcdefgh".to_vec());
        let token = svc.issue("agent-1", Some("swarm-1"), Some("/project"), vec![Capability::Admin], 3600);
        let claims = svc.verify(&token).unwrap();
        assert_eq!(claims.agent_id, "agent-1");
        assert_eq!(claims.swarm_id.as_deref(), Some("swarm-1"));
        assert!(claims.is_admin());
    }

    #[test]
    fn reject_tampered_payload() {
        let svc = CapabilityTokenService::new(b"test-secret-key-32bytes!abcdefgh".to_vec());
        let token = svc.issue("agent-1", None, None, vec![Capability::SwarmRead], 3600);
        // Tamper with payload
        let mut parts: Vec<&str> = token.rsplitn(2, '.').collect();
        parts.reverse();
        let tampered = format!("dGFtcGVyZWQ.{}", parts[1]);
        assert!(matches!(svc.verify(&tampered), Err(TokenError::InvalidSignature)));
    }

    #[test]
    fn reject_wrong_key() {
        let svc1 = CapabilityTokenService::new(b"key-one-32bytes!abcdefghijklmnop".to_vec());
        let svc2 = CapabilityTokenService::new(b"key-two-32bytes!abcdefghijklmnop".to_vec());
        let token = svc1.issue("agent-1", None, None, vec![Capability::Admin], 3600);
        assert!(matches!(svc2.verify(&token), Err(TokenError::InvalidSignature)));
    }

    #[test]
    fn reject_expired_token() {
        let svc = CapabilityTokenService::new(b"test-secret-key-32bytes!abcdefgh".to_vec());
        // Issue with 0 TTL (already expired)
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let token_data = AgentCapabilityToken {
            agent_id: "agent-1".into(),
            swarm_id: None,
            project_dir: None,
            capabilities: vec![Capability::Admin],
            issued_at: now - 100,
            expires_at: now - 50, // expired 50 seconds ago
        };
        let payload = serde_json::to_vec(&token_data).unwrap();
        let mut mac = HmacSha256::new_from_slice(b"test-secret-key-32bytes!abcdefgh").unwrap();
        mac.update(&payload);
        let sig = mac.finalize().into_bytes().to_vec();
        let token = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&payload),
            URL_SAFE_NO_PAD.encode(&sig)
        );
        assert!(matches!(svc.verify(&token), Err(TokenError::Expired)));
    }

    #[test]
    fn capability_checks() {
        let claims = VerifiedClaims {
            agent_id: "a1".into(),
            swarm_id: None,
            project_dir: None,
            capabilities: vec![
                Capability::TaskWrite { task_ids: vec!["t1".into(), "t2".into()] },
                Capability::FileSystem { roots: vec!["/project/src".into()], read_only: false },
                Capability::Memory { scopes: vec!["swarm:s1".into()] },
            ],
        };
        assert!(claims.can_write_task("t1"));
        assert!(!claims.can_write_task("t3"));
        assert!(claims.can_access_path(std::path::Path::new("/project/src/main.rs"), true));
        assert!(!claims.can_access_path(std::path::Path::new("/etc/passwd"), false));
        assert!(claims.can_access_memory("swarm:s1:key"));
        assert!(!claims.can_access_memory("global:key"));
        assert!(!claims.is_admin());
    }
}
