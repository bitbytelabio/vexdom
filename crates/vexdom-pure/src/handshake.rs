//! The extension handshake's crypto (`docs/research/handshake-crypto.md`): the daemon's gate and the
//! extension (through ext-core) compute the same proofs from this module. Nonces are minted by the
//! caller (`vexdom_core::handshake::new_nonce`, `newNonce` in the extension).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// PBKDF2 rounds over a pairing code: makes grinding a recorded handshake hopeless.
pub const PAIRING_ITERATIONS: u32 = 250_000;

/// What both sides bind their proofs to. `protocol_version` is the extension's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub protocol_version: u32,
    pub extension_version: String,
    pub manifest_hash: String,
    pub client_nonce: String,
    pub server_nonce: String,
}

/// WebCrypto refuses a zero-length HMAC key; the TypeScript greet failed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptySecret;

impl std::fmt::Display for EmptySecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Zero-length key is not supported")
    }
}

impl std::error::Error for EmptySecret {}

/// `isNonce`: at least 16 UTF-16 code units.
pub fn is_nonce(value: &str) -> bool {
    value.encode_utf16().count() >= 16
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The secret a pairing code proves: base64url of PBKDF2-HMAC-SHA256 over the two nonces.
pub fn pairing_secret(code: &str, transcript: &Transcript) -> String {
    let salt = format!(
        "vexdom/pair|{}|{}",
        transcript.client_nonce, transcript.server_nonce
    );
    let mut derived = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(
        code.as_bytes(),
        salt.as_bytes(),
        PAIRING_ITERATIONS,
        &mut derived,
    );
    b64(&derived)
}

/// HMAC keyed by the secret's UTF-8 text (not its decoded bytes) over `label|T|extra`.
fn tag(
    secret: &str,
    label: &str,
    transcript: &Transcript,
    extra: &str,
) -> Result<[u8; 32], EmptySecret> {
    if secret.is_empty() {
        return Err(EmptySecret);
    }
    let message = format!(
        "{label}|{}|{}|{}|{}|{}|{extra}",
        transcript.protocol_version,
        transcript.extension_version,
        transcript.manifest_hash,
        transcript.client_nonce,
        transcript.server_nonce,
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| EmptySecret)?;
    mac.update(message.as_bytes());
    Ok(mac.finalize().into_bytes().into())
}

pub fn client_proof(secret: &str, transcript: &Transcript) -> Result<String, EmptySecret> {
    Ok(b64(&tag(secret, "vexdom/client", transcript, "")?))
}

/// Covers the whole welcome, so none of it can be altered on the way.
pub fn server_proof(
    secret: &str,
    transcript: &Transcript,
    daemon_version: &str,
    manifest_hash: &str,
    manifest_in_sync: bool,
    sealed_session_key: Option<&str>,
) -> Result<String, EmptySecret> {
    let claims = format!(
        "{daemon_version}|{manifest_hash}|{manifest_in_sync}|{}",
        sealed_session_key.unwrap_or("")
    );
    Ok(b64(&tag(secret, "vexdom/server", transcript, &claims)?))
}

/// XOR with a keystream whose block `n` is `HMAC(secret, "vexdom/seal|T|n")`, `n` in decimal.
pub fn seal_session_key(
    secret: &str,
    transcript: &Transcript,
    session_key: &str,
) -> Result<String, EmptySecret> {
    let clear = session_key.as_bytes();
    let mut out = Vec::with_capacity(clear.len());
    for (block, chunk) in clear.chunks(32).enumerate() {
        let stream = tag(secret, "vexdom/seal", transcript, &block.to_string())?;
        out.extend(chunk.iter().zip(stream).map(|(byte, key)| byte ^ key));
    }
    if clear.is_empty() {
        // The TS still imports the key, so an empty secret fails even for an empty session key.
        tag(secret, "vexdom/seal", transcript, "0")?;
    }
    Ok(b64(&out))
}

/// `openSessionKey`: the extension's half of [`seal_session_key`] (tests and tools).
pub fn open_session_key(secret: &str, transcript: &Transcript, sealed: &str) -> Option<String> {
    let cipher = URL_SAFE_NO_PAD.decode(sealed).ok()?;
    let mut clear = Vec::with_capacity(cipher.len());
    for (block, chunk) in cipher.chunks(32).enumerate() {
        let stream = tag(secret, "vexdom/seal", transcript, &block.to_string()).ok()?;
        clear.extend(chunk.iter().zip(stream).map(|(byte, key)| byte ^ key));
    }
    String::from_utf8(clear).ok()
}

/// `sameProof`: equal UTF-16 length, then a constant-time compare.
pub fn same_proof(offered: &str, expected: &str) -> bool {
    offered.len() == expected.len() && bool::from(offered.as_bytes().ct_eq(expected.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Transcript {
        Transcript {
            protocol_version: 26,
            extension_version: "0.9.2".into(),
            manifest_hash: "1a2b3c4d".into(),
            client_nonce: "AAAAAAAAAAAAAAAAAAAAAA".into(),
            server_nonce: "_____________________w".into(),
        }
    }

    // docs/research/handshake-crypto.md §8.
    #[test]
    fn vectors() {
        let t = t0();
        let s43 = "S".repeat(43);
        let secret = pairing_secret("ABCD2345", &t);
        assert_eq!(secret, "bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto");
        assert_eq!(
            client_proof(&secret, &t).unwrap(),
            "1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU"
        );
        let sealed = seal_session_key(&secret, &t, &s43).unwrap();
        assert_eq!(
            sealed,
            "zR6r4OCmRJs9Cj_HswPDQBM1uXNLBjKZRw0WBCbkVJIHg6IkujXxZwy3FA"
        );
        assert_eq!(
            server_proof(&secret, &t, "0.9.2", "1a2b3c4d", true, Some(&sealed)).unwrap(),
            "YmGvK2x55BPsz5gKdJSmimAg7rXp6T9JjDbjUGMzfDc"
        );
        assert_eq!(
            server_proof(&s43, &t, "0.9.2", "1a2b3c4d", false, None).unwrap(),
            "tSsbrc2OIu5c_C0RY1tEYh2cnmy18O4E0NiDZU0tLTg"
        );
        assert_eq!(
            client_proof(&s43, &t).unwrap(),
            "W75TpCxycM9M3U93T7C46pYi6gDGdFvlMEABBy6bsag"
        );
        assert_eq!(
            seal_session_key(&secret, &t, &"k".repeat(70)).unwrap(),
            "9SaT2NiefKMFMgf_izv7eCsNgUtzPgqhfzUuPB7cbKo_u5ocgg3JXzSPLBjbURPgtLPSFKpPjomURp6Dx1pAUQexEf5Y4A"
        );
        assert_eq!(
            pairing_secret("", &t),
            "qzPSJbBeyqYkcDCGP53PzrhzbJpAq0GVf1EovUpNCKo"
        );
        let blank = Transcript {
            extension_version: String::new(),
            manifest_hash: String::new(),
            ..t.clone()
        };
        assert_eq!(
            client_proof(&s43, &blank).unwrap(),
            "bQIDv-bU6OQ7NjvgzH_sGaCkHq1tlsQCsflzsRQbDNQ"
        );
        let joined = Transcript {
            extension_version: "a,b".into(),
            ..t.clone()
        };
        assert_eq!(
            client_proof(&s43, &joined).unwrap(),
            "pOdvGBlCwZLQoAQARRKYnPl5P_1YFSZykL4aCBsQSBw"
        );
        let replaced = Transcript {
            extension_version: "\u{FFFD}".into(),
            ..t.clone()
        };
        assert_eq!(
            client_proof(&s43, &replaced).unwrap(),
            "R0VmAdUMNhTHUdfs5BBGAC3D6awoJLUSZtbxt5oRu7c"
        );
        assert_eq!(
            open_session_key(&secret, &t, &sealed).as_deref(),
            Some(s43.as_str())
        );
        assert_eq!(client_proof("", &t), Err(EmptySecret));
        assert_eq!(seal_session_key("", &t, ""), Err(EmptySecret));
    }

    #[test]
    fn nonces_and_proofs() {
        assert!(is_nonce(&"😀".repeat(8)));
        assert!(!is_nonce(&"a".repeat(15)));
        let proof = "1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU";
        assert!(same_proof(proof, proof));
        assert!(!same_proof(&format!("{proof}x"), proof));
        assert!(!same_proof(
            "1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsV",
            proof
        ));
    }
}
