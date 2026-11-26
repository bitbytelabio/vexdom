//! Rendering content-script snapshots: golden fixtures, refs across snapshots, and the scenarios of
//! agent-browser's `bidi/snapshot.rs` tests (v0.45.0, f0a9835) rebuilt on the wire format.

use serde_json::{Map, Value, json};
use vexdom_snapshot::{RefEntry, RefKey, RefMap, SnapshotOptions, WireSnapshot, render};

fn fixture(json: &str) -> WireSnapshot {
    serde_json::from_str(json).expect("fixture is a valid wire snapshot")
}

fn checkout() -> WireSnapshot {
    fixture(include_str!("fixtures/checkout.json"))
}

fn frames() -> WireSnapshot {
    fixture(include_str!("fixtures/frames.json"))
}

fn controls() -> WireSnapshot {
    fixture(include_str!("fixtures/controls.json"))
}

fn options(interactive: bool, compact: bool, urls: bool) -> SnapshotOptions {
    SnapshotOptions {
        interactive,
        compact,
        urls,
        ..SnapshotOptions::default()
    }
}

fn render_new(snapshot: &WireSnapshot, opts: &SnapshotOptions) -> String {
    render(snapshot, 0, opts, &mut RefMap::new())
}

/// A document whose root holds `children` (wire nodes without children of their own).
fn page(doc_key: &str, children: Vec<Value>) -> WireSnapshot {
    let indexes: Vec<usize> = (1..=children.len()).collect();
    let mut nodes = vec![json!({"id": 1, "r": "RootWebArea", "n": "Page", "c": indexes})];
    nodes.extend(children);
    serde_json::from_value(json!({
        "v": 1, "docKey": doc_key, "url": "https://x.test/", "title": "Page", "nodes": nodes, "frames": []
    }))
    .expect("a valid wire snapshot")
}

fn leaf(id: u64, role: &str, name: &str) -> Value {
    json!({"id": id, "r": role, "n": name})
}

fn key(frame_id: i64, doc_key: &str, id: u64) -> RefKey {
    RefKey {
        frame_id,
        doc_key: doc_key.to_string(),
        id,
    }
}

const CHECKOUT_FULL: &str = r#"- navigation
  - link "Home" [ref=e1]
  - link "Cart" [ref=e2]
- main
  - heading "Checkout" [level=1, ref=e3]
  - textbox "Email" [required, ref=e4]: a@b.c
  - checkbox "Agree" [checked=false, ref=e5]
  - button [ref=e6] hint="icon:lucide-trash; in:Blue mug"
  - button "Save" [ref=e7]
- generic [ref=e8] clickable [cursor:pointer]
  - StaticText "Shipping is free""#;

const CHECKOUT_INTERACTIVE: &str = r#"- link "Home" [ref=e1]
- link "Cart" [ref=e2]
- heading "Checkout" [level=1, ref=e3]
- textbox "Email" [required, ref=e4]: a@b.c
- checkbox "Agree" [checked=false, ref=e5]
- button [ref=e6] hint="icon:lucide-trash; in:Blue mug"
- button "Save" [ref=e7]
- generic "Shipping is free" [ref=e8] clickable [cursor:pointer]"#;

#[test]
fn checkout_renders_like_agent_browser() {
    let snapshot = checkout();
    assert_eq!(
        render_new(&snapshot, &options(false, false, false)),
        CHECKOUT_FULL
    );
    assert_eq!(
        render_new(&snapshot, &options(false, true, false)),
        CHECKOUT_FULL,
        "every line has a ref, a value or a name"
    );
    assert_eq!(
        render_new(&snapshot, &options(true, false, false)),
        CHECKOUT_INTERACTIVE
    );
    assert_eq!(
        render_new(&snapshot, &options(true, true, false)),
        CHECKOUT_INTERACTIVE
    );
    let with_urls = render_new(&snapshot, &options(false, false, true));
    assert!(
        with_urls.contains("\n  - link \"Home\" [ref=e1, url=https://shop.test/]\n"),
        "{with_urls}"
    );
    assert!(
        with_urls.contains("\n  - link \"Cart\" [ref=e2, url=https://shop.test/cart]\n"),
        "{with_urls}"
    );
}

#[test]
fn checkout_refs_carry_their_keys() {
    let mut refs = RefMap::new();
    let _ = render(&checkout(), 0, &SnapshotOptions::default(), &mut refs);
    let doc = "5d1f8e2a-checkout";
    let entries: Vec<(String, RefKey, String, String)> = refs
        .entries_sorted()
        .into_iter()
        .map(|(ref_id, entry)| (ref_id, entry.key, entry.role, entry.name))
        .collect();
    let expected = [
        ("e1", 3, "link", "Home"),
        ("e2", 4, "link", "Cart"),
        ("e3", 6, "heading", "Checkout"),
        ("e4", 7, "textbox", "Email"),
        ("e5", 8, "checkbox", "Agree"),
        ("e6", 9, "button", ""),
        ("e7", 10, "button", "Save"),
        // The ref keeps the accessible name, not the printed cursor text.
        ("e8", 11, "generic", ""),
    ]
    .map(|(ref_id, id, role, name)| {
        (
            ref_id.to_string(),
            key(0, doc, id),
            role.to_string(),
            name.to_string(),
        )
    });
    assert_eq!(entries, expected);
}

#[test]
fn controls_cover_values_states_and_cursor_rules() {
    let snapshot = controls();
    assert_eq!(
        render_new(&snapshot, &SnapshotOptions::default()),
        r#"- slider "Volume" [ref=e1]: 50
- button "Menu" [expanded=false, ref=e2]
- checkbox "Partial" [checked=mixed, ref=e3]
- radio "Express shipping" [checked=true, ref=e4] clickable [cursor:pointer]
  - StaticText "Express shipping"
- link "Docs" [ref=e5]
- paragraph
  - StaticText "Hello world"
- button "Inner" [ref=e6]
- textbox "Password" [ref=e7]
- heading [level=2]
- Iframe [ref=e8]
- button "No id""#
    );
    assert_eq!(
        render_new(&snapshot, &options(true, false, false)),
        r#"- slider "Volume" [ref=e1]: 50
- button "Menu" [expanded=false, ref=e2]
- checkbox "Partial" [checked=mixed, ref=e3]
- radio "Express shipping" [checked=true, ref=e4] clickable [cursor:pointer]
- link "Docs" [ref=e5]
- button "Inner" [ref=e6]
- textbox "Password" [ref=e7]
- Iframe [ref=e8]"#
    );
    let mut refs = RefMap::new();
    let _ = render(&snapshot, 0, &SnapshotOptions::default(), &mut refs);
    let radio = refs.get("e4").expect("the promoted label has a ref");
    assert_eq!(
        (radio.role.as_str(), radio.name.as_str()),
        ("radio", "Express shipping")
    );
    assert_eq!(
        refs.entries_sorted().len(),
        8,
        "the node without an id has no ref"
    );
}

#[test]
fn stitched_frames_render_under_their_iframe() {
    let snapshot = frames();
    let expected = r#"- button "OK" [ref=e1]
- Iframe "Payment" [ref=e2]
  - button "OK" [ref=e4]
  - link "Terms" [ref=e5]
- Iframe "ads" [ref=e3]"#;
    let mut refs = RefMap::new();
    assert_eq!(
        render(&snapshot, 0, &SnapshotOptions::default(), &mut refs),
        expected
    );
    assert_eq!(
        render_new(&snapshot, &options(true, false, false)),
        expected
    );
    assert!(
        render_new(&snapshot, &options(false, false, true))
            .contains("  - link \"Terms\" [ref=e5, url=https://pay.test/terms]")
    );

    let keys: Vec<(String, RefKey, Option<usize>)> = refs
        .entries_sorted()
        .into_iter()
        .map(|(ref_id, entry)| (ref_id, entry.key, entry.nth))
        .collect();
    assert_eq!(
        keys,
        [
            ("e1".to_string(), key(0, "top-doc", 2), None),
            // Both frames have an empty accessible name; the printed title/name is display only.
            ("e2".to_string(), key(0, "top-doc", 3), Some(0)),
            ("e3".to_string(), key(0, "top-doc", 4), Some(1)),
            ("e4".to_string(), key(7, "child-doc", 2), None),
            ("e5".to_string(), key(7, "child-doc", 5), None),
        ],
        "the two OK buttons are in different frames, so neither gets nth"
    );
}

#[test]
fn a_reloaded_child_frame_loses_only_its_refs() {
    let mut refs = RefMap::new();
    let first = frames();
    let _ = render(&first, 0, &SnapshotOptions::default(), &mut refs);

    let mut reloaded = first.clone();
    let child = reloaded.frames[0]
        .snapshot
        .as_mut()
        .expect("the fixture stitches a child frame");
    child.doc_key = "child-doc-2".to_string();
    let tree = render(&reloaded, 0, &SnapshotOptions::default(), &mut refs);

    assert!(tree.contains("- button \"OK\" [ref=e1]"), "{tree}");
    assert!(tree.contains("  - button \"OK\" [ref=e6]"), "{tree}");
    assert!(tree.contains("  - link \"Terms\" [ref=e7]"), "{tree}");
    assert_eq!(
        refs.get("e6").map(|entry| &entry.key),
        Some(&key(7, "child-doc-2", 2))
    );
    assert!(
        refs.get("e4").is_none(),
        "refs of the old child document are gone"
    );
}

// --- Refs across snapshots (docs/internals/decisions.md ADR-021) ---

#[test]
fn the_same_element_keeps_its_number() {
    let mut refs = RefMap::new();
    let opts = SnapshotOptions::default();
    let first = render(
        &page(
            "doc",
            vec![leaf(2, "button", "OK"), leaf(3, "link", "Docs")],
        ),
        0,
        &opts,
        &mut refs,
    );
    assert_eq!(first, "- button \"OK\" [ref=e1]\n- link \"Docs\" [ref=e2]");
    // Renamed and moved: same content-script id, same ref.
    let second = render(
        &page(
            "doc",
            vec![leaf(3, "link", "Guide"), leaf(2, "button", "Done")],
        ),
        0,
        &opts,
        &mut refs,
    );
    assert_eq!(
        second,
        "- link \"Guide\" [ref=e2]\n- button \"Done\" [ref=e1]"
    );
    assert_eq!(
        refs.get("e1").map(|entry| entry.name.as_str()),
        Some("Done")
    );
}

#[test]
fn refs_survive_insertion() {
    let ok = || leaf(2, "button", "OK");
    let mut refs = RefMap::new();
    let _ = render(
        &page("loader", vec![ok()]),
        0,
        &SnapshotOptions::default(),
        &mut refs,
    );
    let out = render(
        &page("loader", vec![leaf(3, "button", "New"), ok()]),
        0,
        &SnapshotOptions::default(),
        &mut refs,
    );
    assert!(out.contains("button \"OK\" [ref=e1]"), "{out}");
    assert!(out.contains("button \"New\" [ref=e2]"), "{out}");
}

#[test]
fn changed_document_allocates_new_refs_and_rejects_stale_refs() {
    let mut refs = RefMap::new();
    let first = render(
        &page("loader-a", vec![leaf(2, "button", "OK")]),
        0,
        &SnapshotOptions::default(),
        &mut refs,
    );
    assert!(first.contains("button \"OK\" [ref=e1]"), "{first}");
    let stale_ref = refs.entries_sorted()[0].0.clone();

    // The frame navigated: a new content-script instance reuses id 2 for another element.
    let changed = render(
        &page("loader-b", vec![leaf(2, "button", "OK")]),
        0,
        &SnapshotOptions::default(),
        &mut refs,
    );
    assert!(changed.contains("button \"OK\" [ref=e2]"), "{changed}");
    assert!(refs.get(&stale_ref).is_none());
}

#[test]
fn numbers_are_never_recycled() {
    let mut refs = RefMap::new();
    let opts = SnapshotOptions::default();
    let buttons = || vec![leaf(2, "button", "A"), leaf(3, "button", "B")];
    let _ = render(&page("doc-a", buttons()), 0, &opts, &mut refs);
    let _ = render(&page("doc-b", buttons()), 0, &opts, &mut refs);
    // Back to a document key seen before: its numbers were forgotten with it, not reissued.
    let again = render(&page("doc-a", buttons()), 0, &opts, &mut refs);
    assert_eq!(again, "- button \"A\" [ref=e5]\n- button \"B\" [ref=e6]");
    let refs_now: Vec<String> = refs.entries_sorted().into_iter().map(|(r, _)| r).collect();
    assert_eq!(refs_now, ["e5", "e6"]);
}

#[test]
fn a_ref_missing_from_the_latest_snapshot_is_unknown() {
    let mut refs = RefMap::new();
    let opts = SnapshotOptions::default();
    let _ = render(
        &page(
            "doc",
            vec![leaf(2, "button", "Keep"), leaf(3, "button", "Drop")],
        ),
        0,
        &opts,
        &mut refs,
    );
    assert!(refs.get("e2").is_some());
    let _ = render(
        &page("doc", vec![leaf(2, "button", "Keep")]),
        0,
        &opts,
        &mut refs,
    );
    assert!(refs.get("e2").is_none(), "e2 is not in the latest snapshot");
    assert_eq!(
        refs.get("e1"),
        Some(&RefEntry {
            key: key(0, "doc", 2),
            role: "button".to_string(),
            name: "Keep".to_string(),
            nth: None,
        })
    );
    // The element comes back: same document, same id, same number.
    let back = render(
        &page(
            "doc",
            vec![leaf(2, "button", "Keep"), leaf(3, "button", "Drop")],
        ),
        0,
        &opts,
        &mut refs,
    );
    assert_eq!(
        back,
        "- button \"Keep\" [ref=e1]\n- button \"Drop\" [ref=e2]"
    );
}

#[test]
fn refs_sort_numerically_and_keep_that_order_in_json() {
    let mut refs = RefMap::new();
    let links: Vec<Value> = (0..12)
        .map(|i| leaf(100 + i, "link", &format!("L{i}")))
        .collect();
    let _ = render(
        &page("doc", links),
        0,
        &SnapshotOptions::default(),
        &mut refs,
    );
    let expected = [
        "e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9", "e10", "e11", "e12",
    ];
    let sorted = refs.entries_sorted();
    let order: Vec<&str> = sorted.iter().map(|(ref_id, _)| ref_id.as_str()).collect();
    assert_eq!(order, expected);
    // agent-browser's serde_json sorts object keys (e10 before e2); the workspace enables
    // `preserve_order`, so a map built from `entries_sorted` keeps the numeric order.
    let map: Map<String, Value> = sorted
        .into_iter()
        .map(|(ref_id, entry)| (ref_id, json!({"role": entry.role, "name": entry.name})))
        .collect();
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    assert_eq!(keys, expected);
    let text = serde_json::to_string(&map).expect("a JSON object");
    let position = |ref_id: &str| text.find(&format!("\"{ref_id}\"")).expect("ref in JSON");
    assert!(
        position("e2") < position("e10"),
        "numeric order survives serialization: {text}"
    );
}

// --- agent-browser bidi/snapshot.rs scenarios on the wire format ---

#[test]
fn renders_chrome_format() {
    let snapshot = page(
        "loader",
        vec![
            leaf(2, "button", "OK"),
            json!({"id": 3, "r": "textbox", "n": "Email", "v": "a@b.c"}),
            json!({"id": 4, "r": "checkbox", "n": "Agree", "s": {"checked": "true"}}),
            json!({"id": 5, "r": "button", "n": "Send", "s": {"disabled": true}}),
            json!({"r": "StaticText", "n": "hello"}),
        ],
    );
    let out = render_new(&snapshot, &SnapshotOptions::default());
    assert!(out.contains("button \"OK\" [ref=e1]"), "{out}");
    assert!(out.contains("textbox \"Email\" [ref=e2]: a@b.c"), "{out}");
    assert!(
        out.contains("checkbox \"Agree\" [checked=true, ref=e3]"),
        "{out}"
    );
    assert!(out.contains("button \"Send\" [disabled, ref=e4]"), "{out}");
    assert!(out.contains("StaticText \"hello\""), "{out}");
}

#[test]
fn duplicate_buttons_get_distinct_refs_with_nth() {
    let snapshot = page(
        "loader",
        vec![
            json!({"id": 2, "r": "button", "n": "OK", "h": "id:2"}),
            json!({"id": 3, "r": "button", "n": "OK", "h": "id:3"}),
        ],
    );
    let mut map = RefMap::new();
    let out = render(&snapshot, 0, &SnapshotOptions::default(), &mut map);
    assert_eq!(map.get("e1").unwrap().nth, Some(0));
    assert_eq!(map.get("e2").unwrap().nth, Some(1));
    assert!(out.contains(r#"button "OK" [ref=e1] hint="id:2""#), "{out}");
    assert!(out.contains(r#"button "OK" [ref=e2] hint="id:3""#), "{out}");
}

#[test]
fn duplicate_names_in_different_frames_do_not_get_nth() {
    let mut top = page(
        "top",
        vec![leaf(2, "button", "OK"), json!({"id": 3, "r": "Iframe"})],
    );
    let child = page("child", vec![leaf(2, "button", "OK")]);
    top.frames = serde_json::from_value(json!([
        {"node": 2, "frameId": 4, "sameOrigin": false, "snapshot": child}
    ]))
    .expect("a valid frame slot");
    let mut refs = RefMap::new();

    let _ = render(&top, 0, &SnapshotOptions::default(), &mut refs);

    assert_eq!(refs.get("e1").unwrap().nth, None);
    assert_eq!(refs.get("e3").unwrap().nth, None);
    assert_eq!(refs.get("e3").unwrap().key, key(4, "child", 2));
}

#[test]
fn link_targets_show_only_with_urls_and_never_as_values() {
    let snapshot = page(
        "loader",
        vec![
            json!({"id": 5, "r": "link", "n": "Docs", "v": "https://x.test/docs", "u": "https://x.test/docs"}),
        ],
    );
    let render_with = |urls: bool| {
        render_new(
            &snapshot,
            &SnapshotOptions {
                urls,
                ..Default::default()
            },
        )
    };
    assert_eq!(render_with(false), "- link \"Docs\" [ref=e1]");
    assert_eq!(
        render_with(true),
        "- link \"Docs\" [ref=e1, url=https://x.test/docs]"
    );
}

#[test]
fn deep_pages_render_on_a_small_stack() {
    // 20,000 nested wrappers (transparent when rendered) around one button.
    let depth = 20_000;
    let mut nodes = vec![json!({"id": 1, "r": "RootWebArea", "c": [1]})];
    nodes.extend((1..depth).map(|i| json!({"id": i + 1, "r": "generic", "c": [i + 1]})));
    nodes.push(json!({"id": depth + 1, "r": "button", "n": "Deep"}));
    let snapshot: WireSnapshot = serde_json::from_value(json!({
        "v": 1, "docKey": "doc", "url": "https://x.test/", "title": "", "nodes": nodes, "frames": []
    }))
    .expect("a valid wire snapshot");
    let rendered = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || render_new(&snapshot, &SnapshotOptions::default()))
        .expect("a test thread")
        .join()
        .expect("rendering does not recurse per level");
    assert_eq!(rendered, "- button \"Deep\" [ref=e1]");
}

#[test]
fn empty_pages_say_so() {
    let empty = page("doc", vec![json!({"r": "StaticText", "n": " "})]);
    assert_eq!(
        render_new(&empty, &SnapshotOptions::default()),
        "(empty page)"
    );
    assert_eq!(
        render_new(&empty, &options(true, false, false)),
        "(no interactive elements)"
    );
    let mut nothing = empty.clone();
    nothing.nodes.clear();
    assert_eq!(
        render_new(&nothing, &SnapshotOptions::default()),
        "(empty page)"
    );
}
