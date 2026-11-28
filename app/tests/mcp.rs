//! `vexdom mcp` over stdio against an in-process Bridge and a Rust fake extension: the three tools,
//! the playbooks over MCP, and both protocol eras (ADR-029).
#![allow(clippy::unwrap_used)]

mod support;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{Value, json};
use support::fake_extension::{self, Canned};
use support::{Session, paired_bridge, scratch};

const CHECKOUT: &str = include_str!("../../crates/vexdom-snapshot/tests/fixtures/checkout.json");
const MODERN: &str = "2026-07-28";
/// Bytes of the tool list and server instructions every session starts with (~4 per token).
const CATALOG_CEILING: usize = 5_750;

fn golden(name: &str, actual: &Value) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let text = format!("{}\n", serde_json::to_string_pretty(actual).unwrap());
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == text,
        "{name} drifted; rerun with UPDATE_GOLDEN=1 if intended\n{text}"
    );
}

/// The `_meta` a 2026-07-28 client sends with every request.
fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

#[test]
fn stdio_session_drives_the_page_through_refs() {
    let home = scratch("mcp");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, async move {
        let canned: fake_extension::Shared = Rc::new(RefCell::new(Canned::default()));
        {
            let mut page: Value = serde_json::from_str(CHECKOUT).unwrap();
            page["tabId"] = json!(7);
            let mut canned = canned.borrow_mut();
            canned.answers.insert("page.snapshot".into(), page);
            canned.answers.insert(
                "page.clickElement".into(),
                json!({"tag": "button", "role": "button", "selector": "#a > b", "text": "Pay",
                    "bounds": {"x": 1, "y": 2, "width": 3, "height": 4}, "submits": false}),
            );
            canned.answers.insert(
                "page.listSiteTools".into(),
                json!({"pageOrigin": "https://shop.test", "count": 1, "tools": [
                    {"name": "addToCart", "description": "Add an item.\u{200B}", "origin": "https://shop.test", "frameId": 0}
                ]}),
            );
            canned.answers.insert(
                "page.readHtml".into(),
                json!({"url": "https://shop.test/", "title": "Shop", "html": "<h1>Shop</h1><p>Hello</p><h2>Pricing</h2><p>Free tier</p>"}),
            );
        }
        let _daemon = paired_bridge(&canned).await;

        // Legacy era: the initialize handshake.
        let mut mcp = Session::start(&home).await;
        let init = mcp.initialize().await;
        assert_eq!(init["result"]["serverInfo"]["name"], "vexdom");
        for capability in ["tools", "prompts", "resources"] {
            assert!(init["result"]["capabilities"].get(capability).is_some(), "{init}");
        }

        let listed = mcp.request("tools/list", json!({})).await;
        let tools = listed["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|tool| tool["name"].as_str()).collect();
        assert_eq!(names, ["snapshot", "act", "guide"]);
        golden("catalog.json", &listed["result"]["tools"]);
        let catalog = serde_json::to_string(&listed["result"]["tools"]).unwrap().len()
            + init["result"]["instructions"].as_str().map_or(0, str::len);
        assert!(
            catalog <= CATALOG_CEILING,
            "catalog grew past its token budget; trim it or raise CATALOG_CEILING on purpose ({catalog} > {CATALOG_CEILING})"
        );

        let refused = mcp.call("act", json!({"do": "clickElement", "target": "e1"})).await;
        assert!(refused.contains("NO_SNAPSHOT"), "{refused}");

        let first = mcp.call("snapshot", json!({})).await;
        assert!(first.contains("snapshot r1\n") && first.contains("[ref=e1]"), "{first}");
        // The page's WebMCP tools ride along once, page text escaped inside a delimited block.
        assert!(first.contains("provides 1 WebMCP tool, listed below"), "{first}");
        assert!(first.contains("[origin https://shop.test] addToCart [frame 0]: Add an item.\\u{200B}"), "{first}");
        assert!(first.contains(r#"act {"do":"callSiteTool","tool":"<tool>","frameId":0"#), "{first}");

        // find answers matching lines with their refs, and leaves the delta baseline at r1.
        let found = mcp.call("snapshot", json!({"find": "email"})).await;
        assert!(found.contains("find \"email\": 1 of ") && found.contains("textbox \"Email\" [required, ref=e4]"), "{found}");
        assert!(!found.contains("Shipping"), "{found}");

        let clicked = mcp.call("act", json!({"do": "clickElement", "target": "e1"})).await;
        assert!(clicked.contains("\"text\":\"Pay\"") && clicked.contains("snapshot r2: unchanged"), "{clicked}");
        // The agent holds the ref: the element's selector, box and tag are not echoed back.
        assert!(!clicked.contains("selector") && !clicked.contains("bounds") && !clicked.contains("\"tag\""), "{clicked}");
        assert!(!clicked.contains("WebMCP"), "{clicked}");
        let sent = canned.borrow().calls.iter().rfind(|(action, _)| action == "page.clickElement").unwrap().1.clone();
        assert_eq!(sent["target"]["ref"]["role"], "link");

        let read = mcp.call("snapshot", json!({"as": "markdown", "outline": true})).await;
        assert!(read.contains("Shop") && read.contains("Pricing"), "{read}");

        // A URL read goes over HTTP, but only after the browser's Blocked sites list allows it.
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/doc", server.local_addr().unwrap());
        tokio::task::spawn_local(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            while let Ok((mut stream, _)) = server.accept().await {
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request).await;
                let body = "# Docs\n\nRead me. token=sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/markdown\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        let refused = mcp.call("snapshot", json!({"as": "markdown", "url": url})).await;
        assert!(refused.contains("UNKNOWN_ACTION") && !refused.contains("Read me"), "{refused}");
        canned.borrow_mut().answers.insert("page.checkSites".into(), json!({"allowed": true}));
        // An agent reads public pages only, unless the user who started the server allowed more.
        let local = mcp.call("snapshot", json!({"as": "markdown", "url": url})).await;
        assert!(local.contains("READ_REFUSED") && !local.contains("Read me"), "{local}");
        let mut reader = Session::start_with(&home, &[("VEXDOM_READ_PRIVATE", "1")]).await;
        reader.initialize().await;
        let fetched = reader.call("snapshot", json!({"as": "markdown", "url": url})).await;
        assert!(fetched.contains("# Docs") && fetched.contains("Read me"), "{fetched}");
        assert!(!fetched.contains("abcdefghijklmnopqrstuvwxyz0123456789"), "{fetched}");
        let checked = canned.borrow().calls.iter().rfind(|(action, _)| action == "page.checkSites").unwrap().1.clone();
        assert_eq!(checked["urls"][0], url);

        // A batch takes one after-action snapshot, for its last step, and lets the page settle.
        let snapshots = |canned: &fake_extension::Shared| {
            canned.borrow().calls.iter().filter(|(action, _)| action == "page.snapshot").count()
        };
        let before = snapshots(&canned);
        let twice = mcp
            .call("act", json!({"batch": [{"do": "clickElement", "target": "e1"}, {"do": "clickElement", "target": "e1"}]}))
            .await;
        assert_eq!(snapshots(&canned), before + 1, "{twice}");
        assert_eq!(twice.matches("unchanged").count(), 1, "{twice}");
        let calls = canned.borrow().calls.clone();
        let (_, settled) = calls.iter().rfind(|(action, _)| action == "page.snapshot").unwrap();
        assert!(settled["settleMs"].as_u64().is_some_and(|ms| ms > 0), "{settled}");
        assert!(calls.iter().all(|(_, input)| input.get("_noDelta").is_none()), "the Bridge strips _noDelta");

        let batch = mcp
            .call("act", json!({"batch": [{"do": "clickElement", "target": "e1"}, {"do": "clickElement", "target": "e99"}]}))
            .await;
        assert!(batch.contains("UNKNOWN_REF"), "{batch}");

        let unknown = mcp.call("act", json!({"do": "nope"})).await;
        assert!(unknown.contains("UNKNOWN_ACTION"), "{unknown}");

        let guide = mcp.call("guide", json!({})).await;
        assert!(guide.contains("Actions (call act") && guide.contains("vexdom — "), "{guide}");
        let action = mcp.call("guide", json!({"topic": "clickElement"})).await;
        assert!(action.starts_with("clickElement: ") && action.contains("read-only: no"), "{action}");

        let prompts = mcp.request("prompts/list", json!({})).await;
        assert!(prompts["result"]["prompts"].as_array().unwrap().iter().any(|p| p["name"] == "vexdom"), "{prompts}");
        let skill = mcp.request("resources/read", json!({"uri": "skill://vexdom/SKILL.md"})).await;
        let body = skill["result"]["contents"][0]["text"].as_str().unwrap_or("");
        assert!(body.contains("name: vexdom"), "{skill}");

        // 2026-07-28: no handshake, metadata on every request.
        let mut modern = Session::start(&home).await;
        let discovered = modern
            .request("server/discover", json!({"_meta": modern_meta()}))
            .await;
        assert!(
            discovered["result"]["supportedVersions"].as_array().unwrap().iter().any(|v| v == MODERN),
            "{discovered}"
        );
        let listed = modern.request("tools/list", json!({"_meta": modern_meta()})).await;
        assert_eq!(listed["result"]["tools"].as_array().map(Vec::len), Some(3), "{listed}");
        assert_eq!(listed["result"]["resultType"], "complete", "{listed}");
        let skills = modern.request("skills/list", json!({"_meta": modern_meta()})).await;
        assert!(skills.to_string().contains("skill://vexdom/SKILL.md"), "{skills}");

        // An action that needs approval: the client asks its user, then calls again (ADR-032).
        {
            let mut canned = canned.borrow_mut();
            canned.answers.insert("page.submitForm".into(), json!({"submitted": true}));
            canned.answers.insert("page.query".into(), json!({"value": "https://shop.test/cart"}));
        }
        let mut meta = modern_meta();
        meta["io.modelcontextprotocol/clientCapabilities"] = json!({"elicitation": {"form": {}}});
        let asked = modern
            .request(
                "tools/call",
                json!({"_meta": meta, "name": "act", "arguments": {"do": "submitForm", "selector": "form"}}),
            )
            .await;
        assert_eq!(asked["result"]["resultType"], "input_required", "{asked}");
        let state = asked["result"]["requestState"].as_str().unwrap_or("").to_owned();
        assert!(state.starts_with("c-"), "{asked}");
        let message = asked["result"]["inputRequests"]["confirm"]["params"]["message"].as_str().unwrap_or("");
        assert!(message.starts_with("Allow submitForm on shop.test?"), "{asked}");
        let done = modern
            .request(
                "tools/call",
                json!({"_meta": meta, "name": "act", "arguments": {"do": "submitForm", "selector": "form"},
                    "requestState": state,
                    "inputResponses": {"confirm": {"action": "accept", "content": {"decision": "allow"}}}}),
            )
            .await;
        assert_eq!(done["result"]["isError"], false, "{done}");
        assert!(done.to_string().contains("submitted"), "{done}");
        let expired = modern
            .request(
                "tools/call",
                json!({"_meta": meta, "name": "act", "arguments": {}, "requestState": state, "inputResponses": {}}),
            )
            .await;
        assert!(expired.to_string().contains("CONFIRM_EXPIRED"), "{expired}");
    });
}
