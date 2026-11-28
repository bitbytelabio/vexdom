//! The token bench's "after" sessions (plan step 13): `vexdom mcp` against an in-process Bridge whose
//! fake extension answers with what `capture.mjs --after` recorded from the bench pages. Writes
//! bench/tokens/golden-after/mcp/*.jsonl for `count.mjs --after`.
//!
//! `cargo test -p vexdom --test mcp_bench -- --ignored`
#![allow(clippy::unwrap_used)]

mod support;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{Value, json};
use support::fake_extension::{self, Canned};
use support::{Session, paired_bridge, scratch};

fn bench_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bench/tokens/golden-after")
}

fn page(fixture: &str, file: &str) -> Value {
    let path = bench_dir().join("pages").join(fixture).join(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing: run capture.mjs --after", path.display()));
    serde_json::from_str(&text).expect("captured JSON")
}

fn write_session(name: &str, session: &mut Session) {
    let dir = bench_dir().join("mcp");
    std::fs::create_dir_all(&dir).expect("golden-after/mcp");
    let text = format!("{}\n", session.transcript.join("\n"));
    std::fs::write(dir.join(format!("{name}.jsonl")), text).expect("the session is written");
    session.transcript.clear();
}

#[test]
#[ignore = "writes bench/tokens/golden-after; run by hand for the token bench"]
fn bench_sessions() {
    let home = scratch("mcp-bench");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    tokio::task::LocalSet::new().block_on(&runtime, async move {
        let canned: fake_extension::Shared = Rc::new(RefCell::new(Canned::default()));
        let _daemon = paired_bridge(&canned).await;
        let mut mcp = Session::start(&home).await;
        mcp.initialize().await;
        mcp.request("tools/list", json!({})).await;
        write_session("catalog", &mut mcp);

        let index: Value = serde_json::from_str(
            &std::fs::read_to_string(bench_dir().join("pages/index.json"))
                .expect("capture.mjs --after wrote index.json"),
        )
        .expect("index JSON");
        for entry in index.as_array().expect("a list of fixtures") {
            let fixture = entry["fixture"].as_str().expect("a fixture name");
            {
                let mut canned = canned.borrow_mut();
                canned
                    .answers
                    .insert("page.snapshot".into(), page(fixture, "snapshot.json"));
                canned
                    .answers
                    .insert("page.readHtml".into(), page(fixture, "html.json"));
            }
            // Ids 1-3, in the order count.mjs reads them.
            mcp.call("snapshot", json!({})).await;
            mcp.call("snapshot", json!({"as": "markdown"})).await;
            mcp.call("snapshot", json!({"as": "markdown", "outline": true}))
                .await;
            if fixture == "snapshot_diff_probe" {
                canned
                    .borrow_mut()
                    .answers
                    .insert("page.snapshot".into(), page(fixture, "snapshot-after.json"));
                // Id 4: what changed after typing into Account name.
                mcp.call("snapshot", json!({})).await;
            }
            write_session(&format!("calls-{fixture}"), &mut mcp);
        }
    });
}
