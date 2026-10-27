// SPDX-License-Identifier: GPL-3.0-only

//! The `dh-ietf1024-sha256-aes128-cbc-pkcs7` transfer encryption from the
//! Secret Service specification.
//!
//! This protects secrets in transit on the session bus. It is not protection
//! against a compromised session.

use aes::Aes128;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use hkdf::Hkdf;
use num_bigint_dig::BigUint;
use sha2::Sha256;
use zeroize::Zeroizing;

pub const ALGORITHM: &str = "dh-ietf1024-sha256-aes128-cbc-pkcs7";

/// RFC 2409 "Second Oakley Group" (1024-bit MODP), generator 2.
const PRIME_1024_HEX: &[u8] = b"\
FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD1\
29024E088A67CC74020BBEA63B139B22514A08798E3404DD\
EF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245\
E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED\
EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381\
FFFFFFFFFFFFFFFF";
/// RFC 3526 1536-bit MODP group, generator 2. Used by the GNOME prompter
/// protocol's secret exchange (`[sx-aes-1]`).
#[cfg(feature = "builtin-prompter")]
const PRIME_1536_HEX: &[u8] = b"\
FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD1\
29024E088A67CC74020BBEA63B139B22514A08798E3404DD\
EF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245\
E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED\
EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3D\
C2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F\
83655D23DCA3AD961C62F356208552BB9ED529077096966D\
670C354E4ABC9804F1746C08CA237327FFFFFFFFFFFFFFFF";
const PRIVATE_KEY_LEN: usize = 32;
const AES_KEY_LEN: usize = 16;
const IV_LEN: usize = 16;
const BLOCK_LEN: usize = 16;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("random number generator failed")]
    Random,
    #[error("service sent an invalid public key")]
    InvalidPublicKey,
    #[error("service sent invalid encryption parameters")]
    InvalidParameters,
    #[error("secret could not be decrypted")]
    Decrypt,
}

/// A Diffie-Hellman group. Both use generator 2, and the AES key is
/// HKDF-SHA256 (no salt, no info) of the shared secret padded to the
/// prime's length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Secret Service transfer sessions.
    Ietf1024,
    /// The GNOME prompter's secret exchange.
    #[cfg(feature = "builtin-prompter")]
    Ietf1536,
}

impl Group {
    fn prime(self) -> BigUint {
        let hex = match self {
            Self::Ietf1024 => PRIME_1024_HEX,
            #[cfg(feature = "builtin-prompter")]
            Self::Ietf1536 => PRIME_1536_HEX,
        };
        BigUint::parse_bytes(hex, 16).expect("constant prime parses")
    }

    fn len(self) -> usize {
        match self {
            Self::Ietf1024 => 128,
            #[cfg(feature = "builtin-prompter")]
            Self::Ietf1536 => 192,
        }
    }
}

#[cfg(test)]
fn prime() -> BigUint {
    Group::Ietf1024.prime()
}

/// Our half of the key exchange. The private exponent is wiped on drop.
pub struct KeyPair {
    group: Group,
    private: Zeroizing<BigUint>,
    public: Vec<u8>,
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyPair([REDACTED])")
    }
}

impl KeyPair {
    /// A key pair for a Secret Service transfer session.
    pub fn generate() -> Result<Self, CryptoError> {
        Self::generate_in(Group::Ietf1024)
    }

    pub fn generate_in(group: Group) -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new([0u8; PRIVATE_KEY_LEN]);
        getrandom::fill(bytes.as_mut()).map_err(|_| CryptoError::Random)?;
        let private = Zeroizing::new(BigUint::from_bytes_be(bytes.as_ref()));
        let public = BigUint::from(2u32)
            .modpow(&private, &group.prime())
            .to_bytes_be();
        Ok(Self {
            group,
            private,
            public,
        })
    }

    /// The public key to send in `OpenSession`.
    #[must_use]
    pub fn public_key(&self) -> &[u8] {
        &self.public
    }

    /// Derives the AES key from the service's public key.
    pub fn derive_key(&self, service_public: &[u8]) -> Result<SessionKey, CryptoError> {
        let p = self.group.prime();
        let theirs = BigUint::from_bytes_be(service_public);
        // Reject degenerate keys (0, 1, p-1, >= p) that would make the shared
        // secret predictable.
        let one = BigUint::from(1u32);
        if theirs <= one || theirs >= &p - &one {
            return Err(CryptoError::InvalidPublicKey);
        }

        let shared = Zeroizing::new(theirs.modpow(&self.private, &p));
        let shared_bytes = Zeroizing::new(shared.to_bytes_be());
        let len = self.group.len();
        let mut ikm = Zeroizing::new(vec![0u8; len]);
        ikm[len - shared_bytes.len()..].copy_from_slice(&shared_bytes);

        let mut key = Zeroizing::new([0u8; AES_KEY_LEN]);
        Hkdf::<Sha256>::new(None, ikm.as_ref())
            .expand(&[], key.as_mut())
            .map_err(|_| CryptoError::InvalidParameters)?;
        Ok(SessionKey(key))
    }
}

/// The negotiated AES-128 key. Wiped on drop.
pub struct SessionKey(Zeroizing<[u8; AES_KEY_LEN]>);

impl std::fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKey([REDACTED])")
    }
}

impl SessionKey {
    /// Decrypts a secret value. `parameters` is the IV sent by the service.
    /// The plaintext is decrypted in place inside a wiping buffer.
    pub fn decrypt(
        &self,
        parameters: &[u8],
        value: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        if parameters.len() != IV_LEN || value.is_empty() || !value.len().is_multiple_of(BLOCK_LEN)
        {
            return Err(CryptoError::InvalidParameters);
        }
        let mut buffer = Zeroizing::new(value.to_vec());
        let plain_len = cbc::Decryptor::<Aes128>::new_from_slices(self.0.as_ref(), parameters)
            .map_err(|_| CryptoError::InvalidParameters)?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .map_err(|_| CryptoError::Decrypt)?
            .len();
        buffer.truncate(plain_len);
        Ok(buffer)
    }

    /// Encrypts a secret value with a fresh random IV. Returns the IV (sent
    /// as the secret's parameters) and the ciphertext. The plaintext is only
    /// read, never copied.
    pub fn encrypt(&self, plain: &[u8]) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        let mut iv = vec![0u8; IV_LEN];
        getrandom::fill(&mut iv).map_err(|_| CryptoError::Random)?;
        let mut out = vec![0u8; (plain.len() / BLOCK_LEN + 1) * BLOCK_LEN];
        let len = cbc::Encryptor::<Aes128>::new_from_slices(self.0.as_ref(), &iv)
            .map_err(|_| CryptoError::InvalidParameters)?
            .encrypt_padded_b2b::<Pkcs7>(plain, &mut out)
            .map_err(|_| CryptoError::InvalidParameters)?
            .len();
        out.truncate(len);
        Ok((iv, out))
    }

    #[cfg(test)]
    fn bytes(&self) -> &[u8] {
        self.0.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_sides_derive_the_same_key() {
        let a = KeyPair::generate().unwrap();
        let b = KeyPair::generate().unwrap();
        let ka = a.derive_key(b.public_key()).unwrap();
        let kb = b.derive_key(a.public_key()).unwrap();
        assert_eq!(ka.bytes(), kb.bytes());
    }

    #[cfg(feature = "builtin-prompter")]
    #[test]
    fn prompter_group_round_trip() {
        let a = KeyPair::generate_in(Group::Ietf1536).unwrap();
        let b = KeyPair::generate_in(Group::Ietf1536).unwrap();
        assert!(a.public_key().len() > 128);
        let ka = a.derive_key(b.public_key()).unwrap();
        let kb = b.derive_key(a.public_key()).unwrap();
        assert_eq!(ka.bytes(), kb.bytes());
        let (iv, ct) = ka.encrypt(b"KBX-UNIT-SX").unwrap();
        assert_eq!(kb.decrypt(&iv, &ct).unwrap().as_slice(), b"KBX-UNIT-SX");
    }

    #[test]
    fn rejects_degenerate_public_keys() {
        let a = KeyPair::generate().unwrap();
        let p = prime();
        for bad in [
            BigUint::from(0u32),
            BigUint::from(1u32),
            &p - BigUint::from(1u32),
            p.clone(),
        ] {
            assert_eq!(
                a.derive_key(&bad.to_bytes_be()).unwrap_err(),
                CryptoError::InvalidPublicKey
            );
        }
    }

    #[test]
    fn decrypt_round_trip_and_bad_input() {
        let a = KeyPair::generate().unwrap();
        let b = KeyPair::generate().unwrap();
        let key = a.derive_key(b.public_key()).unwrap();
        let iv = [7u8; IV_LEN];
        let plain = b"KBX-UNIT-CRYPTO";
        let mut buf = vec![0u8; 32];
        let n = cbc::Encryptor::<Aes128>::new_from_slices(key.bytes(), &iv)
            .unwrap()
            .encrypt_padded_b2b::<Pkcs7>(plain, &mut buf)
            .unwrap()
            .len();
        buf.truncate(n);

        assert_eq!(key.decrypt(&iv, &buf).unwrap().as_slice(), plain);
        // Malformed input from the service must be an error, never a panic.
        assert_eq!(
            key.decrypt(&iv[..8], &buf).unwrap_err(),
            CryptoError::InvalidParameters
        );
        assert_eq!(
            key.decrypt(&iv, &buf[..5]).unwrap_err(),
            CryptoError::InvalidParameters
        );
        assert_eq!(
            key.decrypt(&iv, &[]).unwrap_err(),
            CryptoError::InvalidParameters
        );
    }

    #[test]
    fn encrypt_round_trip_uses_fresh_ivs() {
        let a = KeyPair::generate().unwrap();
        let b = KeyPair::generate().unwrap();
        let ours = a.derive_key(b.public_key()).unwrap();
        let theirs = b.derive_key(a.public_key()).unwrap();
        for plain in [&b""[..], b"KBX-UNIT-ENCRYPT", &[0u8; 32]] {
            let (iv, ct) = ours.encrypt(plain).unwrap();
            assert_eq!(iv.len(), IV_LEN);
            assert!(!ct.is_empty() && ct.len().is_multiple_of(BLOCK_LEN));
            assert_eq!(theirs.decrypt(&iv, &ct).unwrap().as_slice(), plain);
        }
        let (iv1, _) = ours.encrypt(b"x").unwrap();
        let (iv2, _) = ours.encrypt(b"x").unwrap();
        assert_ne!(iv1, iv2);
    }
}
