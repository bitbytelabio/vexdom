//! The handshake's one impure step: minting nonces. The crypto is `vexdom_pure::handshake`.

/// A fresh 16-byte nonce, base64url.
pub fn new_nonce() -> String {
    crate::rand::base64url(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonces_are_long_enough() {
        assert!(vexdom_pure::handshake::is_nonce(&new_nonce()));
    }
}
