//! The Bridge end to end with a fake extension and a real control client.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};
use support::fake_extension::{self, Canned, Credential, Outcome};
use vexdom_core::{lockfile, paths};
use vexdom_daemon::Daemon;
use vexdom_schema::{action_result, control};

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

async fn control(daemon: &Daemon) -> control::Client {
    let lock = lockfile::read(&paths::lockfile()).expect("the daemon wrote its lockfile");
    assert_eq!(lock.port, daemon.port());
    vexdom_link::connect_control(lock.port, &lock.token)
        .await
        .unwrap()
        .0
}

async fn invoke(
    control: &control::Client,
    action: &str,
    input: Value,
    run_id: &str,
) -> Result<Value, (String, String)> {
    let mut request = control.invoke_request();
    request.get().set_action(action);
    request.get().set_input(input.to_string());
    request.get().set_run_id(run_id);
    let reply = request.send().promise.await.unwrap();
    result_json(reply.get().unwrap().get_result().unwrap())
}

async fn pairing_code(control: &control::Client) -> String {
    let reply = control.pair_request().send().promise.await.unwrap();
    reply
        .get()
        .unwrap()
        .get_code()
        .unwrap()
        .to_string()
        .unwrap()
}

#[test]
fn pairs_reconnects_and_invokes() {
    let _scratch = support::scratch("pairs");
    support::local(async {
        let daemon = Daemon::start(false).await.unwrap();
        let control = control(&daemon).await;
        let canned: fake_extension::Shared = Rc::new(RefCell::new(Canned::default()));
        canned.borrow_mut().answers.insert(
            "page.getPageInfo".into(),
            json!({"title": "Fixture", "url": "https://a.test/"}),
        );

        // A wrong code is refused, retryably, with the TS code.
        let wrong = fake_extension::connect(
            daemon.port(),
            Credential::Code("WRONGCOD"),
            Rc::clone(&canned),
        )
        .await
        .unwrap();
        match wrong {
            Outcome::Unauthorized { code, reason } => {
                assert_eq!(code, "bad-code");
                assert!(reason.starts_with("That pairing code is wrong or expired."));
            }
            Outcome::Welcome { .. } => panic!("a wrong code paired"),
        }

        let code = pairing_code(&control).await;
        let Outcome::Welcome {
            session,
            session_key,
            in_sync,
        } = fake_extension::connect(daemon.port(), Credential::Code(&code), Rc::clone(&canned))
            .await
            .unwrap()
        else {
            panic!("the code did not pair");
        };
        assert!(in_sync);
        let key = session_key.expect("pairing hands over a session key");
        session.ping_request().send().promise.await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let status = control.status_request().send().promise.await.unwrap();
        let status = status.get().unwrap().get_status().unwrap();
        assert!(status.get_connected());
        assert_eq!(status.get_browser().unwrap().to_str().unwrap(), "Chrome");
        assert_eq!(status.get_paired_browsers(), 1);

        let info = invoke(&control, "page.getPageInfo", json!({}), "")
            .await
            .unwrap();
        assert_eq!(info["title"], "Fixture");
        assert_eq!(
            invoke(&control, "vexdom.status", json!({}), "")
                .await
                .unwrap_err()
                .0,
            "BLOCKED"
        );
        assert_eq!(
            invoke(&control, "page.readHtml", json!({}), "")
                .await
                .unwrap_err()
                .0,
            "UNKNOWN_ACTION"
        );
        // Pushes reached the side panel after the welcome.
        assert!(
            canned
                .borrow()
                .pushes
                .iter()
                .any(|push| push == "preferencesInfo")
        );

        // The session key reconnects; the older link is superseded.
        let again =
            fake_extension::connect(daemon.port(), Credential::Session(&key), Rc::clone(&canned))
                .await
                .unwrap();
        assert!(matches!(again, Outcome::Welcome { .. }));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let sessions = control.sessions_request().send().promise.await.unwrap();
        let sessions = sessions.get().unwrap().get_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions.get(0).get_connected());
        assert_eq!(
            sessions.get(0).get_paired_by().unwrap().to_str().unwrap(),
            "code"
        );

        // A stale key is refused as unknown.
        let stale = fake_extension::connect(
            daemon.port(),
            Credential::Session("not-the-key-at-all"),
            canned,
        )
        .await
        .unwrap();
        assert!(matches!(stale, Outcome::Unauthorized { code, .. } if code == "unknown"));
        daemon.stop();
    });
}

#[test]
fn a_run_cannot_loosen_its_guardrails() {
    let _scratch = support::scratch("run-guard");
    support::local(async {
        let daemon = Daemon::start(false).await.unwrap();
        let control = control(&daemon).await;
        let mut describe = control.describe_request();
        describe.get().set_run_id("run-123");
        describe.send().promise.await.unwrap();
        let mut set = control.set_preference_request();
        set.get()
            .set_change(r#"{"kind":"theme","theme":"midnight"}"#);
        let reply = set.send().promise.await.unwrap();
        let (code, message) = result_json(reply.get().unwrap().get_result().unwrap()).unwrap_err();
        assert_eq!(code, "BLOCKED");
        assert!(message.starts_with("An agent run cannot change the settings it runs under."));
        assert_eq!(
            invoke(&control, "page.getPageInfo", json!({}), "run-123")
                .await
                .unwrap_err()
                .0,
            "RUN_INACTIVE"
        );
        let mut pairing = control.pairing_request();
        pairing.get().set_set("code");
        let reply = pairing.send().promise.await.unwrap();
        assert_eq!(
            reply.get().unwrap().get_error().unwrap().to_str().unwrap(),
            "An agent run cannot change how browsers pair."
        );
        // A fresh connection is not a run: it may.
        let other = vexdom_link::connect_control(daemon.port(), daemon.token())
            .await
            .unwrap()
            .0;
        let mut set = other.set_preference_request();
        set.get()
            .set_change(r#"{"kind":"theme","theme":"midnight"}"#);
        let reply = set.send().promise.await.unwrap();
        let prefs = result_json(reply.get().unwrap().get_result().unwrap()).unwrap();
        assert_eq!(prefs["theme"], "midnight");
        daemon.stop();
    });
}

#[test]
fn offline_and_refused_upgrades() {
    let _scratch = support::scratch("offline");
    support::local(async {
        let daemon = Daemon::start(false).await.unwrap();
        let control = control(&daemon).await;
        let (code, message) = invoke(&control, "page.getPageInfo", json!({}), "")
            .await
            .unwrap_err();
        assert_eq!(code, "EXTENSION_OFFLINE");
        assert!(message.starts_with("The Vexdom extension is not connected"));
        assert!(
            vexdom_link::connect_control(daemon.port(), "wrong-token")
                .await
                .is_err()
        );
        let health = vexdom_daemon::ensure::healthy_pid(daemon.port()).await;
        assert_eq!(health, Some(std::process::id()));
        daemon.stop();
    });
}

async fn confirmations(control: &control::Client) -> Vec<Value> {
    let reply = control
        .confirmations_request()
        .send()
        .promise
        .await
        .unwrap();
    let list = reply.get().unwrap().get_list().unwrap().to_str().unwrap();
    serde_json::from_str::<Value>(list)
        .unwrap()
        .as_array()
        .cloned()
        .unwrap()
}

/// The first confirmation to appear, polled.
async fn waiting(control: &control::Client) -> Value {
    for _ in 0..100 {
        if let Some(first) = confirmations(control).await.into_iter().next() {
            return first;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no confirmation appeared");
}

async fn answer(control: &control::Client, id: &str, allow: bool, remember: bool) -> bool {
    let mut request = control.confirm_request();
    request.get().set_id(id);
    request.get().set_allow(allow);
    request.get().set_remember(remember);
    let reply = request.send().promise.await.unwrap();
    reply.get().unwrap().get_found()
}

#[test]
fn external_confirm_waits_for_a_decision() {
    let _scratch = support::scratch("confirm");
    support::local(async {
        let daemon = Daemon::start(false).await.unwrap();
        let control = control(&daemon).await;
        let canned: fake_extension::Shared = Rc::new(RefCell::new(Canned::default()));
        {
            let mut canned = canned.borrow_mut();
            canned
                .answers
                .insert("page.submitForm".into(), json!({"submitted": true}));
            canned.answers.insert(
                "page.query".into(),
                json!({"value": "https://a.test/login"}),
            );
        }
        let code = pairing_code(&control).await;
        let Outcome::Welcome { .. } =
            fake_extension::connect(daemon.port(), Credential::Code(&code), Rc::clone(&canned))
                .await
                .unwrap()
        else {
            panic!("the code did not pair");
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let submit = || {
            let control = control.clone();
            tokio::task::spawn_local(async move {
                invoke(
                    &control,
                    "page.submitForm",
                    json!({"selector": "form", "_approver": "cli:42"}),
                    "",
                )
                .await
            })
        };

        // Allowed: the action runs, without the approver tag.
        let call = submit();
        let pending = waiting(&control).await;
        assert!(pending["id"].as_str().unwrap().starts_with("c-"));
        assert_eq!(pending["action"], "page.submitForm");
        assert_eq!(pending["site"], "a.test");
        assert_eq!(pending["approver"], "cli:42");
        assert!(answer(&control, pending["id"].as_str().unwrap(), true, false).await);
        assert_eq!(call.await.unwrap().unwrap(), json!({"submitted": true}));
        let sent = canned
            .borrow()
            .calls
            .iter()
            .rfind(|(action, _)| action == "page.submitForm")
            .unwrap()
            .1
            .clone();
        assert_eq!(sent, json!({"selector": "form"}));
        assert!(confirmations(&control).await.is_empty());
        assert!(!answer(&control, pending["id"].as_str().unwrap(), true, false).await);

        // Denied.
        let call = submit();
        let pending = waiting(&control).await;
        assert!(answer(&control, pending["id"].as_str().unwrap(), false, false).await);
        assert_eq!(call.await.unwrap().unwrap_err().0, "DECLINED");

        // Always on this site: the next one goes straight through.
        let call = submit();
        let pending = waiting(&control).await;
        assert!(answer(&control, pending["id"].as_str().unwrap(), true, true).await);
        assert!(call.await.unwrap().is_ok());
        assert!(submit().await.unwrap().is_ok());

        // An agent run never answers.
        let mut describe = control.describe_request();
        describe.get().set_run_id("run-9");
        describe.send().promise.await.unwrap();
        let mut request = control.confirm_request();
        request.get().set_id("c-00000000");
        request.get().set_allow(true);
        assert!(request.send().promise.await.is_err());
        daemon.stop();
    });
}
