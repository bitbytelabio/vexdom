//! agent-browser's flow tests that still apply (`native/flow/mod.rs`), on the Bridge's commands
//! (`{"do": ...}`) and tiers (page, replay).

use std::time::Instant;

use serde_json::{Value, json};

use super::outcome::{Failure, RunOutcome, TierResult, finish_run, tier_result};
use super::page_tier::{page_fetch_script, page_result, tab_on_site};
use super::recorder::{SaveArgs, stable_command};
use super::replay::{check_replay_commands, replay_commands, replay_lookup};
use super::send::{WriteLedger, next_tier, tiers_for};
use super::*;

fn entry(id: &str, url: &str, rtype: &str, mime: &str, body: &str) -> HarEntry {
    HarEntry {
        request_id: id.to_string(),
        started_ms: 0.0,
        method: "GET".to_string(),
        url: url.to_string(),
        request_headers: vec![
            ("Accept".to_string(), "application/json".to_string()),
            ("Cookie".to_string(), "session=secret".to_string()),
            ("X-CSRF-Token".to_string(), "abc".to_string()),
        ],
        post_data: None,
        resource_type: rtype.to_string(),
        status: Some(200),
        mime_type: mime.to_string(),
        response_headers: Vec::new(),
        response_body: Some(body.to_string()),
        response_body_base64: false,
        finished: true,
    }
}

fn recorder() -> FlowRecorder {
    let mut r = FlowRecorder::default();
    r.begin_capture(None);
    let nav = json!({ "do": "navigate", "url": "https://www.example.com.ar/" });
    let seq = r.begin_command_at(1000.0, "page.navigate", &nav).unwrap();
    r.finish_command(seq, true, 1100.0);
    r.tag("doc", 1010.0);
    r.tag("api", 1200.0);
    r.tag("ga", 1300.0);
    let click = json!({ "do": "clickElement", "target": { "role": "button", "name": "Go" } });
    let seq = r
        .begin_command_at(5000.0, "page.clickElement", &click)
        .unwrap();
    r.finish_command(seq, true, 5100.0);
    r
}

fn save_input<'a>(
    rec: &'a FlowRecorder,
    entries: &'a [HarEntry],
    params: &'a [(String, String)],
) -> SaveInput<'a> {
    SaveInput {
        name: "showtimes",
        recorder: rec,
        commands: &rec.commands,
        description: None,
        entries,
        params,
        output: None,
        created_at: "2026-11-26T00:00:00Z".to_string(),
        login_wall: false,
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vexdom-flow-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn non_post_graphql_query_requests_remain_writes() {
    for body in [
        r#"{"query":"query Items { items { id } }"}"#,
        r#"[{"query":"{ items { id } }"}]"#,
    ] {
        assert!(!is_write("POST", Some(body)));
        for method in ["PUT", "PATCH", "DELETE"] {
            assert!(is_write(method, Some(body)), "{method} must remain a write");
        }
    }
    assert!(is_write("POST", Some(r#"{"query":"mutation { buy }"}"#)));
}

#[test]
fn base_domain_handles_second_level_country_domains() {
    assert_eq!(base_domain("api.example.com.ar"), "example.com.ar");
    assert_eq!(base_domain("www.example.org"), "example.org");
    assert_eq!(base_domain("127.0.0.1"), "127.0.0.1");
}

#[test]
fn save_keeps_first_party_json_and_drops_credentials() {
    let rec = recorder();
    let entries = vec![
        entry(
            "doc",
            "https://www.example.com.ar/",
            "Document",
            "text/html",
            "<html>",
        ),
        entry(
            "api",
            "https://api.example.com.ar/movies?film=Film%20One",
            "Fetch",
            "application/json",
            r#"{"movies":[{"id":1}],"total":1}"#,
        ),
        entry(
            "ga",
            "https://www.google-analytics.com/collect",
            "XHR",
            "application/json",
            "{}",
        ),
    ];
    let params = vec![("film".to_string(), "Film One".to_string())];
    let (flow, report) = build_flow(save_input(&rec, &entries, &params)).unwrap();
    let flow = serde_json::to_value(&flow).unwrap();
    let req = &flow["steps"][0]["requests"][0];
    assert_eq!(
        req["url"],
        "https://api.example.com.ar/movies?film={{film}}"
    );
    assert!(req["headers"].get("Cookie").is_none());
    assert!(req["headers"].get("X-CSRF-Token").is_none());
    assert_eq!(req["expect"]["strippedAuth"], true);
    assert_eq!(req["expect"]["list"], "movies");
    assert_eq!(flow["output"], "s1r1");
    assert_eq!(flow["allowedHosts"], json!(["example.com.ar"]));
    assert_eq!(flow["createdBy"]["tool"], "vexdom");
    assert!(!flow.to_string().contains("secret"));
    assert_eq!(report["gaps"][0]["step"], 2);
    assert_eq!(
        report["gaps"][0]["reason"],
        "no network traffic while this command ran"
    );
}

#[test]
fn save_fails_without_api_traffic() {
    let rec = recorder();
    let entries = vec![entry(
        "doc",
        "https://www.example.com.ar/",
        "Document",
        "text/html",
        "<html>",
    )];
    let err = build_flow(save_input(&rec, &entries, &[])).unwrap_err();
    assert!(err.contains("no first-party JSON requests"), "{err}");
}

#[test]
fn a_request_belongs_to_the_command_whose_window_holds_its_start() {
    let rec = recorder();
    assert_eq!(rec.request_tags.get("api"), Some(&1));
    // 1300 ms is within 1.5 s of the navigation's end.
    assert_eq!(rec.request_tags.get("ga"), Some(&1));
    let mut rec = recorder();
    rec.tag("late", 2700.0);
    assert!(
        !rec.request_tags.contains_key("late"),
        "past the grace, no step"
    );
    rec.untag_between(6000.0, 6100.0);
    rec.tag("screenshot", 6050.0);
    assert!(!rec.request_tags.contains_key("screenshot"));
}

#[test]
fn reads_and_capture_tools_are_not_steps() {
    let mut r = FlowRecorder::default();
    r.begin_capture(None);
    assert!(
        r.record("page.query", &json!({"what": "url"}), 1.0, 2.0, true)
            .is_none()
    );
    assert!(
        r.record("page.harStop", &json!({}), 1.0, 2.0, true)
            .is_none()
    );
    assert!(
        r.record("page.cookies", &json!({"op": "get"}), 1.0, 2.0, true)
            .is_none()
    );
    assert_eq!(
        r.record("page.waitForElement", &json!({}), 1.0, 2.0, true),
        Some(1)
    );
    assert_eq!(
        r.record("page.clickElement", &json!({}), 3.0, 4.0, false),
        Some(2)
    );
    assert!(
        r.is_ok(1) && !r.is_ok(2),
        "failed steps are kept and flagged"
    );
    assert_eq!(r.end_capture(), 2);
    assert!(
        r.record("page.clickElement", &json!({}), 5.0, 6.0, true)
            .is_none()
    );
}

#[test]
fn a_ref_is_stored_as_its_role_and_name() {
    let input = json!({
        "target": { "ref": { "frameId": 0, "doc": "d1", "id": 7, "role": "button", "name": "Load", "nth": 1 } },
        "tabId": 4,
    });
    let cmd = stable_command("page.clickElement", &input);
    assert_eq!(
        cmd,
        json!({ "do": "clickElement", "target": { "role": "button", "name": "Load", "exact": true, "nth": 1 } })
    );
}

#[test]
fn a_typed_value_becomes_a_param_and_a_secret_is_redacted() {
    let mut r = FlowRecorder::default();
    r.begin_capture(Some("https://a.test/".into()));
    let email = json!({ "target": { "label": "Email" }, "value": "ada@a.test" });
    r.record("page.fillInput", &email, 1.0, 2.0, true);
    assert_eq!(r.commands[0].1["value"], "{{email}}");
    let password =
        json!({ "target": { "role": "textbox", "name": "Password" }, "value": "hunter2" });
    r.record("page.fillInput", &password, 3.0, 4.0, true);
    assert_eq!(r.commands[1].1["value"], REDACTED);
    assert_eq!(r.commands[1].1["redacted"], true);
    assert!(r.secrets.contains(&"hunter2".to_string()));
    let sealed = json!({ "target": { "label": "Card" }, "value": "⟦password:1#abcdef12⟧" });
    r.record("page.fillInput", &sealed, 5.0, 6.0, true);
    assert_eq!(r.commands[2].1["redacted"], true);
    assert!(r.unknown_secret_seqs.contains(&3));
}

#[test]
fn keys_pressed_in_a_secret_field_are_gathered_as_one_secret() {
    let mut r = FlowRecorder::default();
    r.begin_capture(None);
    r.record(
        "page.focusInput",
        &json!({ "target": { "label": "Password" } }),
        1.0,
        2.0,
        true,
    );
    for (at, key) in [(3.0, "a"), (5.0, "b")] {
        r.record("page.pressKey", &json!({ "key": key }), at, at + 1.0, true);
    }
    r.record(
        "page.pressKey",
        &json!({ "key": "1", "modifiers": ["shift"] }),
        7.0,
        8.0,
        true,
    );
    assert_eq!(r.secrets.last().map(String::as_str), Some("ab!"));
    assert_eq!(r.commands[1].1["key"], REDACTED);
}

#[test]
fn a_save_starts_at_the_last_navigation_after_the_previous_save() {
    let mut r = FlowRecorder::default();
    r.begin_capture(None);
    for (i, action) in ["navigate", "clickElement", "navigate", "clickElement"]
        .iter()
        .enumerate()
    {
        let at = i as f64 * 10.0;
        r.record(
            &format!("page.{action}"),
            &json!({ "url": "https://a.test/" }),
            at,
            at + 1.0,
            true,
        );
    }
    assert_eq!(r.resolve_range(None, None).unwrap(), (3, 4));
    assert_eq!(r.resolve_range(Some(2), Some(3)).unwrap(), (2, 3));
    r.mark_saved(4);
    assert_eq!(r.unsaved_steps(), 0);
    assert!(r.resolve_range(None, None).is_err());
    assert!(r.resolve_range(None, Some(9)).is_err());
}

#[test]
fn signature_detects_removed_and_retyped_keys() {
    let rec = signature(&json!({ "movies": [], "total": 1 }));
    assert!(
        signature_matches(
            &rec,
            &signature(&json!({ "movies": [], "total": 2, "extra": true }))
        )
        .is_ok()
    );
    assert!(
        signature_matches(&rec, &signature(&json!({ "total": 2 })))
            .unwrap_err()
            .contains("movies")
    );
    assert!(signature_matches(&rec, &signature(&json!([1]))).is_err());
    let nested = signature(&json!({ "data": [{ "movieName": "x" }] }));
    let moved = signature(&json!({ "data": [{ "title": "x" }] }));
    assert!(
        signature_matches(&nested, &moved)
            .unwrap_err()
            .contains("data[].movieName")
    );
}

#[test]
fn shape_filters_by_substring_and_picks_fields() {
    let body = json!({ "data": [
        { "movieName": "FILM ALPHA", "room": "5", "occupation": { "free": 10 } },
        { "movieName": "FILM BETA", "room": "2", "occupation": { "free": 3 } },
    ] });
    let shape = Shape::from_cmd(
        &json!({ "where": { "movieName": "alpha" }, "fields": ["room", "occupation.free"] }),
    );
    let out = apply_shape(body, &shape);
    assert_eq!(out["total"], 2);
    assert_eq!(out["matched"], 1);
    assert_eq!(
        out["items"][0],
        json!({ "room": "5", "occupation.free": 10 })
    );
}

#[test]
fn prepare_fills_params_and_rejects_unknown_ones() {
    let flow = flow_of(json!({
        "params": [{ "name": "film", "example": "Film One" }],
        "output": "s1r1",
        "steps": [{ "requests": [{ "id": "s1r1", "method": "GET", "url": "https://a.test/m?film={{film}}" }] }],
    }));
    let reqs = prepare(&flow, &[("film".to_string(), "Film Two".to_string())]).unwrap();
    assert_eq!(reqs[0].url, "https://a.test/m?film=Film%20Two");
    assert!(prepare(&flow, &[("city".to_string(), "x".to_string())]).is_err());
    let write = flow_of(json!({
        "output": "s1r1",
        "steps": [{ "requests": [{ "id": "s1r1", "method": "PUT", "url": "https://a.test/items" }] }],
    }));
    assert!(
        prepare(&write, &[])
            .unwrap_err()
            .contains("does not list it")
    );
}

fn prepared(id: &str) -> PreparedRequest {
    PreparedRequest {
        id: id.to_string(),
        step: 1,
        method: "GET".to_string(),
        url: "https://a.test/m".to_string(),
        headers: vec![
            ("User-Agent".to_string(), "x".to_string()),
            ("country".to_string(), "AR".to_string()),
        ],
        body: None,
        expect: serde_json::from_value(
            json!({ "signature": signature(&json!({ "data": [{ "id": 1 }] })) }),
        )
        .unwrap(),
        write: false,
        parts: None,
        repeated_headers: Vec::new(),
    }
}

fn attempt(tier: Tier, outcome: Result<(u16, String), Failure>) -> TierResult {
    tier_result(&prepared("s1r1"), tier, Instant::now(), outcome)
}

#[test]
fn a_tier_that_got_a_page_instead_of_data_falls_back_and_the_flow_stays_verified() {
    let mut flow = flow_of(json!({
        "output": "s1r1",
        "steps": [{ "requests": [{ "id": "s1r1", "method": "GET", "url": "https://a.test/x" }] }],
    }));
    let attempts = vec![
        attempt(Tier::Page, Ok((200, "<html>sign in</html>".to_string()))),
        attempt(
            Tier::Replay,
            Ok((200, r#"{"data":[{"id":1}]}"#.to_string())),
        ),
    ];
    let out = finish_run(&mut flow, &attempts, "now", 1, &Shape::default());
    assert_eq!(out["verified"], true);
    assert_eq!(out["steps"][0]["tier"], "replay");
    assert_eq!(out["steps"][0]["fellBack"][0]["tier"], "page");
    assert_eq!(flow.verified_at.as_deref(), Some("now"));
    assert_eq!(tiers_for(&flow, "s1r1"), vec![Tier::Replay]);
}

#[test]
fn a_list_that_moved_is_drift_with_its_own_code() {
    let mut req = prepared("s1r1");
    req.expect = serde_json::from_value(json!({
        "signature": signature(&json!({ "items": [{ "name": "x" }], "total": 1 })),
    }))
    .unwrap();
    let moved = r#"{"data":{"products":[{"title":"x"}]}}"#;
    let result = tier_result(
        &req,
        Tier::Page,
        Instant::now(),
        Ok((200, moved.to_string())),
    );
    assert!(result.shape_changed);
    let outcome = RunOutcome::failed(&[result], &flow_of(json!({})));
    assert_eq!(outcome.code(), "FLOW_DRIFTED");
    let message = outcome.message("catalog");
    assert!(message.contains("save the flow again"), "{message}");
}

#[test]
fn a_run_outcome_names_what_stopped_the_run() {
    let denied = attempt(Tier::Page, Ok((401, "{}".to_string())));
    let outcome = RunOutcome::stopped(&[denied]).unwrap();
    assert_eq!(outcome.code(), "AUTH_EXPIRED");
    assert!(
        outcome
            .message("f")
            .starts_with("flow 'f' step 1 (page): HTTP 401")
    );
    let limited = attempt(Tier::Page, Ok((429, "{}".to_string())));
    assert_eq!(
        RunOutcome::stopped(&[limited]).unwrap().code(),
        "RATE_LIMITED"
    );
    let challenge = attempt(
        Tier::Page,
        Ok((403, "<html>cf-chl captcha</html>".to_string())),
    );
    assert_eq!(
        RunOutcome::stopped(&[challenge]).unwrap().code(),
        "BLOCKED_BY_SITE"
    );
}

#[test]
fn next_tier_follows_the_order_and_never_resends_a_write() {
    let flow = flow_of(json!({
        "output": "s1r1",
        "writes": [{ "id": "s1r1", "method": "POST", "url": "https://a.test/buy", "kind": "explicit" }],
        "steps": [{ "requests": [{ "id": "s1r1", "method": "POST", "url": "https://a.test/buy" }] }],
    }));
    let mut req = prepared("s1r1");
    req.write = true;
    let ledger = WriteLedger::default();
    assert_eq!(next_tier(&flow, &req, &[], &ledger), Some(Tier::Page));
    let unknown = TierResult::fail(
        &req,
        Tier::Page,
        Instant::now(),
        Failure::unknown_outcome("x".into()),
    );
    assert_eq!(next_tier(&flow, &req, &[unknown], &ledger), None);
}

#[test]
fn page_tier_reads_the_fetch_and_needs_a_tab_on_the_site() {
    let flow = flow_of(json!({ "startUrl": "https://a.test/" }));
    assert!(tab_on_site(&flow, Some("https://www.a.test/x")).is_ok());
    assert!(
        tab_on_site(&flow, Some("https://b.test/"))
            .unwrap_err()
            .contains("not on the flow's site")
    );
    let req = prepared("s1r1");
    let script = page_fetch_script(&req, std::time::Duration::from_secs(5));
    assert!(
        script.contains("\"country\":\"AR\"") && !script.contains("User-Agent"),
        "{script}"
    );
    let policy = HostPolicy::of(&flow);
    let ok = page_result(
        &policy,
        &req,
        Instant::now(),
        Ok(json!({ "status": 200, "url": "https://a.test/m", "text": r#"{"data":[{"id":2}]}"# })),
    );
    assert!(ok.ok, "{:?}", ok.reason);
    let off = vexdom_core::result::ActionError {
        code: "USER_SCRIPTS_DISABLED".into(),
        message: "x".into(),
    };
    assert!(page_result(&policy, &req, Instant::now(), Err(off)).skipped);
}

#[test]
fn replay_swaps_params_and_refuses_secrets_scripts_and_other_sites() {
    let flow = flow_of(json!({
        "params": [{ "name": "q", "example": "tea" }],
        "steps": [
            { "command": { "do": "navigate", "url": "https://a.test/search?q=tea" } },
            { "ok": false, "command": { "do": "clickElement", "target": { "selector": "#gone" } } },
            { "command": { "do": "clickElement", "target": { "selector": "#go" }, "bogus": 1 } },
        ],
    }));
    let cmds = replay_commands(&flow, &[("q".to_string(), "green tea".to_string())]).unwrap();
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0]["url"], "https://a.test/search?q=green%20tea");
    assert!(
        cmds[1].get("bogus").is_none(),
        "only declared fields are replayed"
    );
    assert!(check_replay_commands(&flow, &cmds).is_ok());
    let script = vec![json!({ "do": "waitFor", "fn": "() => true" })];
    assert!(
        check_replay_commands(&flow, &script)
            .unwrap_err()
            .contains("script")
    );
    let away = vec![json!({ "do": "navigate", "url": "https://evil.test/" })];
    assert!(check_replay_commands(&flow, &away).is_err());
    let tabs = vec![json!({ "do": "openTab", "url": "https://a.test/" })];
    assert!(
        check_replay_commands(&flow, &tabs)
            .unwrap_err()
            .contains("not allowed")
    );
    let secret = flow_of(
        json!({ "steps": [{ "command": { "do": "fillInput", "value": REDACTED, "redacted": true } }] }),
    );
    assert!(
        replay_commands(&secret, &[])
            .unwrap_err()
            .contains("secret")
    );
}

#[test]
fn replay_match_ignores_cache_busters() {
    let req = prepared("s1r1");
    let entries = vec![entry(
        "h1",
        "https://a.test/m?_=123",
        "Fetch",
        "application/json",
        "{}",
    )];
    assert!(replay_lookup(&entries, &req).is_ok());
    let other = vec![entry(
        "h1",
        "https://a.test/other",
        "Fetch",
        "application/json",
        "{}",
    )];
    assert!(replay_lookup(&other, &req).is_err());
}

#[test]
fn allow_write_takes_all_or_ids() {
    assert!(AllowWrite::parse(Some(&json!("all"))).unwrap().all);
    assert_eq!(
        AllowWrite::parse(Some(&json!(["s1r1", "s2w1"])))
            .unwrap()
            .ids,
        vec!["s1r1", "s2w1"]
    );
    assert!(AllowWrite::parse(Some(&json!(""))).is_err());
    assert_eq!(AllowWrite::parse(None).unwrap(), AllowWrite::default());
}

#[test]
fn flows_round_trip_privately_and_refuse_unknown_fields() {
    let dir = scratch("store");
    let flow =
        flow_of(json!({ "name": "items", "origin": "a.test", "startUrl": "https://a.test/" }));
    let path = write_flow_in(&dir, &flow).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(dir.join("a.test"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    assert_eq!(load_flow(&dir, "items", None).unwrap().name, "items");
    assert_eq!(list_flows(&dir, Some("www.a.test"))[0]["name"], "items");
    assert_eq!(
        load_flow(&dir, "nope", None).unwrap_err().0,
        "FLOW_NOT_FOUND"
    );
    let mut text: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    text["surprise"] = json!(true);
    std::fs::write(&path, text.to_string()).unwrap();
    assert!(
        load_flow(&dir, "items", None)
            .unwrap_err()
            .1
            .contains("surprise")
    );
    assert!(
        write_flow_in(&dir, &flow_of(json!({ "origin": "../escape" })))
            .unwrap_err()
            .contains("invalid flow origin")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stored_http_tier_reads_as_page() {
    let flow = flow_of(json!({
        "steps": [{ "requests": [{ "id": "s1r1", "method": "GET", "url": "https://a.test/x", "tier": "http" }] }],
    }));
    assert_eq!(flow.steps[0].requests[0].tier, Some(Tier::Page));
}

#[test]
fn har_entries_read_the_extensions_har() {
    let har = json!({ "log": { "entries": [{
        "startedDateTime": "2026-11-26T10:00:00.500Z",
        "request": { "method": "GET", "url": "https://a.test/api/items.json", "headers": [{ "name": "Accept", "value": "*/*" }] },
        "response": { "status": 200, "statusText": "OK", "headers": [], "content": { "mimeType": "application/json", "text": "{\"items\":[]}" } },
        "_resourceType": "Fetch",
    }, {
        "startedDateTime": "2026-11-26T10:00:01.000Z",
        "request": { "method": "GET", "url": "https://a.test/slow", "headers": [] },
        "response": { "status": 0, "statusText": "", "headers": [], "content": { "mimeType": "application/octet-stream" } },
        "_resourceType": "XHR",
    }] } });
    let mut next = 0;
    let entries = har_entries(&har, &mut next);
    assert_eq!(entries[0].request_id, "h1");
    assert_eq!(entries[1].started_ms - entries[0].started_ms, 500.0);
    assert_eq!(entries[0].response_body.as_deref(), Some("{\"items\":[]}"));
    assert!(entries[0].finished && !entries[1].finished);
    assert_eq!(entries[1].mime_type, "", "no headers yet");
}

#[test]
fn values_are_encoded_like_urlencoding() {
    assert_eq!(url_encode("a b/é~"), "a%20b%2F%C3%A9~");
    assert_eq!(url_decode("a%20b%2F%C3%A9").as_deref(), Some("a b/é"));
    assert!(url_decode("%FF").is_none());
}

#[test]
fn a_sealed_handle_counts_as_a_secret() {
    assert!(holds_secret(r#"{"password":"⟦password:3#abcd1234⟧"}"#, &[]));
    assert!(!holds_secret("plain", &[]));
}

#[test]
fn saving_writes_the_flow_and_marks_the_range() {
    let dir = scratch("save");
    let mut rec = recorder();
    let entries = vec![entry(
        "api",
        "https://api.example.com.ar/movies",
        "Fetch",
        "application/json",
        r#"{"movies":[{"id":1}]}"#,
    )];
    let report = rec
        .save(
            &dir,
            SaveArgs {
                name: "movies",
                from: None,
                to: None,
                description: None,
                params: &[],
                output: None,
                entries: &entries,
                login_wall: false,
            },
        )
        .unwrap();
    assert_eq!(report["from"], 1);
    assert_eq!(rec.last_save_seq, 2);
    assert!(dir.join("www.example.com.ar").join("movies.json").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
