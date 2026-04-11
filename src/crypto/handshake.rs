use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey};

use super::stream::CryptoStream;

type HmacSha256 = Hmac<Sha256>;

#[allow(dead_code)]
pub struct HandshakeResult {
    pub crypto: CryptoStream,
    pub peer_authenticated: bool,
}

#[allow(dead_code)]
pub struct KeyPair {
    pub secret: EphemeralSecret,
    pub public: PublicKey,
}

#[allow(dead_code)]
impl KeyPair {
    pub fn generate() -> Self {
        let secret = EphemeralSecret::random_from_rng(rand::thread_rng());
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.public.as_bytes())
    }
}

#[allow(dead_code)]
pub fn decode_public_key(b64: &str) -> Result<PublicKey, HandshakeError> {
    let bytes = BASE64.decode(b64).map_err(|_| HandshakeError::InvalidKey)?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| HandshakeError::InvalidKey)?;
    Ok(PublicKey::from(arr))
}

#[allow(dead_code)]
pub fn derive_shared_secret(secret: EphemeralSecret, peer_public: &PublicKey) -> [u8; 32] {
    let shared = secret.diffie_hellman(peer_public);
    *shared.as_bytes()
}

pub fn create_auth_proof(password: &str, nonce: &[u8], shared_secret: &[u8; 32]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(password.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(nonce);
    mac.update(shared_secret);
    mac.finalize().into_bytes().to_vec()
}

pub fn verify_auth_proof(
    password: &str,
    nonce: &[u8],
    shared_secret: &[u8; 32],
    proof: &[u8],
) -> bool {
    let expected = create_auth_proof(password, nonce, shared_secret);
    expected == proof
}

pub fn generate_nonce() -> Vec<u8> {
    let mut nonce = vec![0u8; 32];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut nonce);
    nonce
}

/// Short hex fingerprint of a shared secret for visual MITM verification.
pub fn fingerprint(shared_secret: &[u8; 32]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(shared_secret);
    format!("{:02x}{:02x}{:02x}", hash[0], hash[1], hash[2])
}

#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("invalid public key")]
    InvalidKey,
    #[error("authentication failed")]
    AuthFailed,
    #[error("unexpected message: {0}")]
    UnexpectedMessage(String),
    #[error("connection error: {0}")]
    Connection(String),
}
