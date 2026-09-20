//! The numeric comparison code shown on both devices while pairing.

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// `HMAC-SHA256(key = nonce, msg = acceptor_fp || requester_fp)`, the first
/// four bytes as a big-endian integer modulo one million, shown as two groups
/// of three digits. The acceptor is the side that received the `PairRequest`.
pub fn pairing_code(nonce: &[u8; 16], acceptor_fp: &str, requester_fp: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(nonce).expect("hmac accepts any key length");
    mac.update(acceptor_fp.as_bytes());
    mac.update(requester_fp.as_bytes());
    let out = mac.finalize().into_bytes();
    let n = u32::from_be_bytes([out[0], out[1], out[2], out[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
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

    const NONCE: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const FP_A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const FP_B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn code_is_six_digits_with_a_space() {
        let code = pairing_code(&NONCE, FP_A, FP_B);
        assert_eq!(code.len(), 7);
        let (hi, lo) = code.split_once(' ').unwrap();
        assert_eq!(hi.len(), 3);
        assert_eq!(lo.len(), 3);
        assert!(hi.chars().chain(lo.chars()).all(|c| c.is_ascii_digit()));
        // Stable across runs: the same inputs always produce the same code.
        assert_eq!(code, pairing_code(&NONCE, FP_A, FP_B));
    }

    #[test]
    fn code_differs_when_roles_swap() {
        let a_first = pairing_code(&NONCE, FP_A, FP_B);
        let b_first = pairing_code(&NONCE, FP_B, FP_A);
        assert_ne!(a_first, b_first);
        assert_eq!(a_first, "137 000");
        assert_eq!(b_first, "129 494");
    }

    #[test]
    fn nonce_hex_round_trips() {
        let hex = nonce_to_hex(&NONCE);
        assert_eq!(hex.len(), 32);
        assert_eq!(nonce_from_hex(&hex), Some(NONCE));
        assert_eq!(nonce_from_hex("zz"), None);
    }
}
