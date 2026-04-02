use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use std::sync::atomic::{AtomicU64, Ordering};

#[allow(dead_code)]
pub struct CryptoStream {
    send_cipher: ChaCha20Poly1305,
    recv_cipher: ChaCha20Poly1305,
    send_nonce_prefix: [u8; 4],
    recv_nonce_prefix: [u8; 4],
    send_counter: AtomicU64,
    recv_counter: AtomicU64,
}

#[allow(dead_code)]
impl CryptoStream {
    pub fn from_shared_secret(shared_secret: &[u8; 32], is_server: bool) -> Self {
        let hk = Hkdf::<Sha256>::new(Some(b"drift-c2s"), shared_secret);
        let mut c2s_key = [0u8; 32];
        hk.expand(b"ws-connector", &mut c2s_key).unwrap();

        let hk = Hkdf::<Sha256>::new(Some(b"drift-s2c"), shared_secret);
        let mut s2c_key = [0u8; 32];
        hk.expand(b"ws-connector", &mut s2c_key).unwrap();

        // Derive nonce prefixes
        let hk = Hkdf::<Sha256>::new(Some(b"drift-nonce"), shared_secret);
        let mut nonce_material = [0u8; 8];
        hk.expand(b"nonce-prefix", &mut nonce_material).unwrap();

        let c2s_prefix: [u8; 4] = nonce_material[..4].try_into().unwrap();
        let s2c_prefix: [u8; 4] = nonce_material[4..8].try_into().unwrap();

        let (send_key, recv_key, send_prefix, recv_prefix) = if is_server {
            (s2c_key, c2s_key, s2c_prefix, c2s_prefix)
        } else {
            (c2s_key, s2c_key, c2s_prefix, s2c_prefix)
        };

        Self {
            send_cipher: ChaCha20Poly1305::new_from_slice(&send_key).unwrap(),
            recv_cipher: ChaCha20Poly1305::new_from_slice(&recv_key).unwrap(),
            send_nonce_prefix: send_prefix,
            recv_nonce_prefix: recv_prefix,
            send_counter: AtomicU64::new(0),
            recv_counter: AtomicU64::new(0),
        }
    }

    fn make_nonce(prefix: &[u8; 4], counter: u64) -> Nonce {
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(prefix);
        nonce[4..12].copy_from_slice(&counter.to_be_bytes());
        *Nonce::from_slice(&nonce)
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let counter = self.send_counter.fetch_add(1, Ordering::SeqCst);
        let nonce = Self::make_nonce(&self.send_nonce_prefix, counter);
        self.send_cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| CryptoError::EncryptionFailed)
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let counter = self.recv_counter.fetch_add(1, Ordering::SeqCst);
        let nonce = Self::make_nonce(&self.recv_nonce_prefix, counter);
        self.recv_cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| CryptoError::DecryptionFailed)
    }
}

#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed")]
    DecryptionFailed,
}
