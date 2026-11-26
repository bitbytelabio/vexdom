//! OS randomness in the shapes the TypeScript Bridge produced (`docs/research/handshake-crypto.md` §5).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// `randomBytes(n)`.
pub fn bytes(len: usize) -> Vec<u8> {
    let mut out = vec![0; len];
    getrandom::fill(&mut out).expect("the OS random source failed");
    out
}

/// `randomBytes(n).toString('base64url')`.
pub fn base64url(len: usize) -> String {
    URL_SAFE_NO_PAD.encode(bytes(len))
}

/// `randomBytes(n).toString('hex')`.
pub fn hex(len: usize) -> String {
    bytes(len)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Node's `randomInt(range)`: 48-bit big-endian draws, rejection without modulo bias.
pub fn int(range: u64) -> u64 {
    const RAND_MAX: u64 = 0xFFFF_FFFF_FFFF;
    let limit = RAND_MAX - (RAND_MAX % range);
    loop {
        let mut draw = [0u8; 8];
        getrandom::fill(&mut draw[2..]).expect("the OS random source failed");
        let value = u64::from_be_bytes(draw);
        if value < limit {
            return value % range;
        }
    }
}

/// `CODE_ALPHABET` of `auth-store.ts`: no I, L, O, U, V, 0 or 1.
pub const CODE_ALPHABET: &[u8; 29] = b"ABCDEFGHJKMNPQRSTWXYZ23456789";

/// `len` characters drawn from [`CODE_ALPHABET`].
pub fn code(len: usize) -> String {
    (0..len)
        .map(|_| CODE_ALPHABET[int(CODE_ALPHABET.len() as u64) as usize] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        assert_eq!(base64url(16).len(), 22);
        assert_eq!(base64url(24).len(), 32);
        assert_eq!(hex(6).len(), 12);
        let code = code(8);
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)));
        assert!((0..1000).all(|_| int(29) < 29));
    }
}
