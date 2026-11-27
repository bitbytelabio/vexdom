//! Snapshot, refs, deltas, read and screenshot flags through the control socket (plan step 8).

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use support::fake_extension::{self, Canned, Credential, Outcome};
use vexdom_daemon::Daemon;
use vexdom_schema::{action_result, control};

const CHECKOUT: &str = include_str!("../../vexdom-snapshot/tests/fixtures/checkout.json");

fn result_json(reader: action_result::Reader<'_>) -> Result<Value, (String, String)> {
    match reader.which().unwrap() {
        action_result::Ok(text) => {
            Ok(serde_json::from_str(text.unwrap().to_str().unwrap()).unwrap())
        }
        action_result::Error(error) => {
            let error = error.unwrap();
            Err((
                error.get_code().unwrap().to_string().unwrap(),
                error.get_message().unwrap().to_string().unwrap(),
            ))
        }
    }
}

async fn invoke(
    control: &control::Client,
    action: &str,
    input: Value,
) -> (Result<Value, (String, String)>, Vec<u8>) {
    let mut request = control.invoke_request();
    request.get().set_action(action);
    request.get().set_input(input.to_string());
    let reply = request.send().promise.await.unwrap();
    let reply = reply.get().unwrap();
    (
        result_json(reply.get_result().unwrap()),
        reply.get_image().unwrap().to_vec(),
    )
}

async fn snapshot(control: &control::Client, full: bool) -> Result<String, String> {
    let mut request = control.snapshot_request();
    {
        let mut opts = request.get().init_opts();
        opts.set_interactive(true);
        opts.set_depth(-1);
        opts.set_tab_id(-1);
        opts.set_full(full);
    }
    match request.send().promise.await {
        Ok(reply) => Ok(reply
            .get()
            .unwrap()
            .get_text()
            .unwrap()
            .to_string()
            .unwrap()),
        Err(error) => Err(error.to_string()),
    }
}

#[test]
fn snapshot_refs_and_deltas() {
    let _scratch = support::scratch("page");
    support::local(async {
        let daemon = Daemon::start(false).await.unwrap();
        let control = vexdom_link::connect_control(daemon.port(), daemon.token())
            .await
            .unwrap()
            .0;
        let canned: fake_extension::Shared = Rc::new(RefCell::new(Canned::default()));
        let mut page: Value = serde_json::from_str(CHECKOUT).unwrap();
        page["tabId"] = json!(7);
        {
            let mut canned = canned.borrow_mut();
            canned.answers.insert("page.snapshot".into(), page);
            canned
                .answers
                .insert("page.clickElement".into(), json!({"clicked": true}));
            canned.answers.insert(
                "page.readHtml".into(),
                json!({"url": "https://shop.test/", "title": "Shop", "html": "<h1>Shop</h1><p>Hello</p><h2>Pricing</h2><p>Free tier</p>"}),
            );
        }
        let pair = control.pair_request().send().promise.await.unwrap();
        let code = pair.get().unwrap().get_code().unwrap().to_string().unwrap();
        let Outcome::Welcome { .. } =
            fake_extension::connect(daemon.port(), Credential::Code(&code), Rc::clone(&canned))
                .await
                .unwrap()
        else {
            panic!("pairing failed");
        };
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        // No snapshot yet: a ref has nothing to name.
        let (refused, _) = invoke(&control, "page.clickElement", json!({"target": "@e1"})).await;
        assert_eq!(
            refused.unwrap_err(),
            ("NO_SNAPSHOT".into(), "Take a snapshot first.".into())
        );

        let first = snapshot(&control, false).await.unwrap();
        assert!(first.starts_with("snapshot r1\n"), "{first}");
        assert!(first.contains("[ref=e1]"), "{first}");
        let again = snapshot(&control, false).await.unwrap();
        assert_eq!(again, "snapshot r2: unchanged");

        // A ref is rewritten to the element it names; the reply carries the delta.
        let (clicked, _) = invoke(&control, "page.clickElement", json!({"target": "e1"})).await;
        let clicked = clicked.unwrap();
        assert_eq!(clicked["result"], json!({"clicked": true}));
        assert_eq!(clicked["snapshot"], "snapshot r3: unchanged");
        let sent = canned
            .borrow()
            .calls
            .iter()
            .find(|(action, _)| action == "page.clickElement")
            .map(|(_, input)| input.clone())
            .unwrap();
        assert_eq!(sent["target"]["ref"]["doc"], "5d1f8e2a-checkout");
        assert_eq!(sent["target"]["ref"]["frameId"], 0);
        assert_eq!(sent["target"]["ref"]["role"], "link");

        let (unknown, _) = invoke(
            &control,
            "page.clickElement",
            json!({"target": {"ref": "e999"}}),
        )
        .await;
        assert_eq!(
            unknown.unwrap_err(),
            (
                "UNKNOWN_REF".into(),
                "Unknown ref: e999. Take a new snapshot.".into()
            )
        );

        // A plain string target is a selector.
        let _ = invoke(&control, "page.clickElement", json!({"target": "#save"})).await;
        let last = canned
            .borrow()
            .calls
            .iter()
            .rfind(|(action, _)| action == "page.clickElement")
            .cloned()
            .unwrap();
        assert_eq!(last.1["target"], json!({"selector": "#save"}));

        // Read: markdown, fenced, outline.
        let mut read = control.read_request();
        read.get().init_opts().set_outline(true);
        let outline = read.send().promise.await.unwrap();
        let outline = outline
            .get()
            .unwrap()
            .get_text()
            .unwrap()
            .to_string()
            .unwrap();
        assert!(outline.starts_with("<<<untrusted-page-data:"), "{outline}");
        assert!(
            outline.contains("1: # Shop\n") && outline.contains(": ## Pricing"),
            "{outline}"
        );
        assert!(!outline.contains("Hello"));

        // Read: a page from an offset names the next one.
        let mut read = control.read_request();
        {
            let mut opts = read.get().init_opts();
            opts.set_offset(2);
            opts.set_max_chars(5);
        }
        let paged = read.send().promise.await.unwrap();
        let paged = paged
            .get()
            .unwrap()
            .get_text()
            .unwrap()
            .to_string()
            .unwrap();
        assert!(paged.starts_with("<<<untrusted-page-data:"), "{paged}");
        assert!(paged.contains("[truncated: chars 2–7 of "), "{paged}");
        assert!(paged.contains("read on with --offset 7]"), "{paged}");

        // Screenshots: bytes travel as Data; an unchanged one is skipped.
        let (shot, image) = invoke(&control, "page.screenshot", json!({"ifChanged": true})).await;
        assert_eq!(shot.unwrap()["format"], "png");
        assert_eq!(image, b"\x89PNG fake");
        let (same, image) = invoke(&control, "page.screenshot", json!({"ifChanged": true})).await;
        assert_eq!(
            same.unwrap(),
            json!("Screenshot unchanged since the last one.")
        );
        assert!(image.is_empty());
        let sent = canned.borrow().calls.last().cloned().unwrap();
        assert!(
            sent.1.get("ifChanged").is_none(),
            "daemon-side flags are stripped"
        );
        daemon.stop();
    });
}
