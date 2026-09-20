//! The numeric comparison code shown on both devices while pairing.
//!
//! Both sides contribute a nonce and the requester commits to its nonce
//! before seeing the acceptor's, so an active relay cannot grind a nonce
//! that makes two unrelated pairings show the same code:
//!
//! 1. Requester B sends `PairRequest { commit = SHA-256(nonce_b) }`.
//! 2. Acceptor A answers `PairChallenge { nonce_a }`.
//! 3. B sends `PairReveal { nonce_b }`; A checks it against the commitment.
//! 4. Both compute `HMAC-SHA256(key = nonce_a || nonce_b, msg = fp_a || fp_b)`,
//!    take the first four bytes as a big-endian integer modulo one million,
//!    and show it as two groups of three digits.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// The acceptor is the side that received the `PairRequest`.
pub fn pairing_code(
    nonce_acceptor: &[u8; 16],
    nonce_requester: &[u8; 16],
    acceptor_fp: &str,
    requester_fp: &str,
) -> String {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(nonce_acceptor);
    key[16..].copy_from_slice(nonce_requester);
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("hmac accepts any key length");
    mac.update(acceptor_fp.as_bytes());
    mac.update(requester_fp.as_bytes());
    let out = mac.finalize().into_bytes();
    let n = u32::from_be_bytes([out[0], out[1], out[2], out[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

/// Hex SHA-256 of a nonce, sent before the nonce itself.
pub fn commitment(nonce: &[u8; 16]) -> String {
    hex::encode(Sha256::digest(nonce))
}

pub fn verify_commitment(nonce: &[u8; 16], commit: &str) -> bool {
    commitment(nonce) == commit
}

pub fn random_nonce() -> [u8; 16] {
    rand::random()
}

pub fn nonce_to_hex(nonce: &[u8; 16]) -> String {
    hex::encode(nonce)
}

pub fn nonce_from_hex(s: &str) -> Option<[u8; 16]> {
    let bytes = hex::decode(s).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE_A: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const NONCE_B: [u8; 16] = [
        0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        0x00,
    ];
    const FP_A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const FP_B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn code_is_six_digits_with_a_space() {
        let code = pairing_code(&NONCE_A, &NONCE_B, FP_A, FP_B);
        assert_eq!(code.len(), 7);
        let (hi, lo) = code.split_once(' ').unwrap();
        assert_eq!(hi.len(), 3);
        assert_eq!(lo.len(), 3);
        assert!(hi.chars().chain(lo.chars()).all(|c| c.is_ascii_digit()));
        assert_eq!(code, pairing_code(&NONCE_A, &NONCE_B, FP_A, FP_B));
    }

    #[test]
    fn code_differs_when_roles_swap() {
        let a_accepts = pairing_code(&NONCE_A, &NONCE_B, FP_A, FP_B);
        let b_accepts = pairing_code(&NONCE_B, &NONCE_A, FP_B, FP_A);
        assert_ne!(a_accepts, b_accepts);
        assert_eq!(a_accepts, "709 597");
        assert_eq!(b_accepts, "844 049");
    }

    #[test]
    fn both_nonces_matter() {
        let base = pairing_code(&NONCE_A, &NONCE_B, FP_A, FP_B);
        let mut other = NONCE_B;
        other[15] ^= 1;
        assert_ne!(base, pairing_code(&NONCE_A, &other, FP_A, FP_B));
        let mut other = NONCE_A;
        other[0] ^= 1;
        assert_ne!(base, pairing_code(&other, &NONCE_B, FP_A, FP_B));
    }

    #[test]
    fn commitment_is_sha256_and_a_mismatch_is_rejected() {
        assert_eq!(
            commitment(&NONCE_A),
            "a8faed6abbf35c12a4b26e40f6feb19d736d90045c83b9f9a31f638d323e6811"
        );
        assert!(verify_commitment(&NONCE_A, &commitment(&NONCE_A)));
        assert!(!verify_commitment(&NONCE_B, &commitment(&NONCE_A)));
        assert!(!verify_commitment(&NONCE_A, ""));
    }

    #[test]
    fn nonce_hex_round_trips() {
        let hex = nonce_to_hex(&NONCE_A);
        assert_eq!(hex.len(), 32);
        assert_eq!(nonce_from_hex(&hex), Some(NONCE_A));
        assert_eq!(nonce_from_hex("zz"), None);
    }
}
