//! The P9 exports as the extension calls them: UTF-16 offsets, time zones by name, JSON shapes.

use ext_core::{
    HandshakeTranscript, HandshakeWelcome, SecretRange, handshake_client_proof,
    handshake_open_session_key, handshake_pairing_secret, handshake_seal_session_key,
    handshake_server_proof, schedule_describe_moment, schedule_next_after, schedule_validate_task,
    secrets_find,
};
use serde_json::{Value, json};

const KEY: &str = "sk-ant-abcdefghijklmnopqrstuvwxyz0123";

#[test]
fn secret_offsets_are_utf16_both_ways() {
    // "é" is one UTF-16 unit and two UTF-8 bytes; "🔑" two units and four bytes.
    let text = format!("é🔑 {KEY} and é🔑 {KEY}");
    let units: Vec<u16> = text.encode_utf16().collect();
    let spans = secrets_find(text.clone(), Vec::new());
    assert_eq!(spans.len(), 2);
    for span in &spans {
        let found = String::from_utf16(&units[span.start as usize..span.end as usize]).unwrap();
        assert_eq!(found, KEY);
        assert_eq!(
            (span.kind.as_str(), span.shape.as_str()),
            ("api-key", "anthropic-key")
        );
        assert_eq!((span.reveal_head, span.reveal_tail), (7, 0));
    }
    // An immune range given in UTF-16 units covers the first key only.
    let first = SecretRange {
        start: spans[0].start,
        end: spans[0].end,
    };
    let rest = secrets_find(text, vec![first]);
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].start, spans[1].start);
}

#[test]
fn schedules_run_on_the_named_zone_across_daylight_saving() {
    // Saturday 2026-03-28 12:00 in Berlin (CET); clocks go forward at 02:00 on Sunday.
    let saturday_noon = 1_774_695_600_000.0;
    let daily = json!({"kind": "weekly", "days": [0, 1, 2, 3, 4, 5, 6], "times": ["09:00"]});
    let next = schedule_next_after(daily.to_string(), saturday_noon, "Europe/Berlin".into())
        .expect("a next run");
    assert_eq!(next - saturday_noon, 20.0 * 3_600_000.0);
    assert_eq!(
        schedule_describe_moment(next, "Europe/Berlin".into()),
        "Sun 29 Mar at 09:00"
    );
    // An unknown zone is UTC.
    assert_eq!(
        schedule_describe_moment(0.0, "Nowhere/Atlantis".into()),
        "Thu 1 Jan at 00:00"
    );
}

#[test]
fn task_validation_keeps_the_typescript_shape() {
    let ok: Value = serde_json::from_str(&schedule_validate_task(
        json!({"name": " T ", "job": {"kind": "instruction", "text": "do"}, "url": "https://a.com",
            "rule": {"kind": "every", "minutes": 10}})
        .to_string(),
        0.0,
        None,
    ))
    .unwrap();
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["draft"]["name"], "T");
    let refused: Value =
        serde_json::from_str(&schedule_validate_task("not json".into(), 0.0, None)).unwrap();
    assert_eq!(
        refused,
        json!({"ok": false, "field": "task", "message": "Nothing to save."})
    );
}

// docs/research/handshake-crypto.md §8.
#[test]
fn handshake_vectors() {
    let t = || HandshakeTranscript {
        protocol_version: 26,
        extension_version: "0.9.2".into(),
        manifest_hash: "1a2b3c4d".into(),
        client_nonce: "AAAAAAAAAAAAAAAAAAAAAA".into(),
        server_nonce: "_____________________w".into(),
    };
    let secret = handshake_pairing_secret("ABCD2345".into(), t());
    assert_eq!(secret, "bHj22qBUKo2-HZgUPOTc6SiIHf8Y4m1UmHtixx8fEto");
    assert_eq!(
        handshake_client_proof(secret.clone(), t()).as_deref(),
        Some("1IrbKWjEXM59Shtx7gMpzLbuqQiaKAG8RJadvd1QtsU")
    );
    let s43 = "S".repeat(43);
    let sealed = handshake_seal_session_key(secret.clone(), t(), s43.clone()).unwrap();
    assert_eq!(
        sealed,
        "zR6r4OCmRJs9Cj_HswPDQBM1uXNLBjKZRw0WBCbkVJIHg6IkujXxZwy3FA"
    );
    let welcome = HandshakeWelcome {
        daemon_version: "0.9.2".into(),
        manifest_hash: "1a2b3c4d".into(),
        manifest_in_sync: true,
        sealed_session_key: Some(sealed.clone()),
    };
    assert_eq!(
        handshake_server_proof(secret.clone(), t(), welcome).as_deref(),
        Some("YmGvK2x55BPsz5gKdJSmimAg7rXp6T9JjDbjUGMzfDc")
    );
    assert_eq!(
        handshake_open_session_key(secret, t(), sealed).as_deref(),
        Some(s43.as_str())
    );
    assert_eq!(handshake_client_proof(String::new(), t()), None);
}
