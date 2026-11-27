//! ext-core driven as `socket.ts` drives it (feed, pump, events), natively, against an in-process
//! Bridge: the handshake with a pairing code, then a page call and a session request.

use std::time::Duration;

use ext_core::{ExtensionRequest, Hello, LinkEvent};
use futures::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use vexdom_core::handshake::new_nonce;
use vexdom_pure::handshake::{self, Transcript};

const ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Sends what the link wrote, then reads frames into it until an event arrives.
async fn next_events(ws: &mut Socket) -> Vec<LinkEvent> {
    loop {
        let out = ext_core::link_pump();
        if !out.is_empty() {
            ws.send(Message::Binary(out.into()))
                .await
                .expect("the Bridge takes the frame");
        }
        let events = ext_core::link_events();
        if !events.is_empty() {
            return events;
        }
        let frame = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("the Bridge answers in time")
            .expect("the socket is open")
            .expect("a frame");
        if let Message::Binary(bytes) = frame {
            ext_core::link_feed(&bytes);
        }
    }
}

#[allow(unsafe_code)]
fn scratch() {
    let home = std::env::temp_dir().join(format!("vexdom-ext-core-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    // SAFETY: this binary has one test, so nothing reads the environment concurrently.
    unsafe {
        std::env::set_var("VEXDOM_HOME", &home);
        std::env::set_var("VEXDOM_PORTS", "0");
        std::env::set_var("HOME", &home);
    }
}

#[test]
fn pairs_answers_a_page_call_and_makes_a_request() {
    scratch();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    tokio::task::LocalSet::new().block_on(&runtime, async {
        let daemon = vexdom_daemon::Daemon::start(false)
            .await
            .expect("the Bridge starts");
        let (control, connection) = vexdom_link::connect_control(daemon.port(), daemon.token())
            .await
            .expect("control");
        tokio::task::spawn_local(connection);
        let pair = control.pair_request().send().promise.await.expect("a code");
        let code = pair
            .get()
            .expect("reply")
            .get_code()
            .expect("code")
            .to_string()
            .expect("utf-8");

        let mut request = format!("ws://127.0.0.1:{}/extension", daemon.port())
            .into_client_request()
            .expect("url");
        request
            .headers_mut()
            .insert("origin", ORIGIN.parse().expect("header"));
        let (mut ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("the socket opens");

        let manifest_hash = vexdom_core::tools::chromium().hash.clone();
        let client_nonce = new_nonce();
        ext_core::link_open();
        ext_core::link_hello(Hello {
            protocol_version: 27,
            min_daemon_protocol: 27,
            extension_version: "0.9.2".into(),
            manifest_hash: manifest_hash.clone(),
            auth: "pair".into(),
            nonce: client_nonce.clone(),
            install_id: "install-0123456789ab".into(),
            browser: "Chrome".into(),
        });
        let [LinkEvent::ServerNonce { nonce }] = &next_events(&mut ws).await[..] else {
            panic!("hello is answered with a nonce")
        };
        let transcript = Transcript {
            protocol_version: 27,
            extension_version: "0.9.2".into(),
            manifest_hash,
            client_nonce,
            server_nonce: nonce.clone(),
        };
        let secret = handshake::pairing_secret(&code, &transcript);
        ext_core::link_prove(handshake::client_proof(&secret, &transcript).expect("a proof"));
        let events = next_events(&mut ws).await;
        let Some(LinkEvent::Welcome {
            protocol_version,
            proof,
            sealed_session_key,
            ..
        }) = events.first()
        else {
            panic!("the code is welcomed: {events:?}")
        };
        assert_eq!(
            *protocol_version,
            vexdom_core::proto::SOCKET_PROTOCOL_VERSION
        );
        assert!(
            !proof.is_empty() && !sealed_session_key.is_empty(),
            "a pairing welcome seals a session key"
        );

        // The Bridge calls the page; JavaScript's answer reaches the control client.
        let invoked = tokio::task::spawn_local({
            let control = control.clone();
            async move {
                let mut request = control.invoke_request();
                request.get().set_action("page.getPageInfo");
                request.get().set_input("{}");
                let reply = request.send().promise.await.expect("an answer");
                let result = reply.get().expect("reply").get_result().expect("result");
                match result.which().expect("a union") {
                    vexdom_schema::action_result::Ok(data) => {
                        data.expect("text").to_string().expect("utf-8")
                    }
                    vexdom_schema::action_result::Error(_) => panic!("the page answered"),
                }
            }
        });
        // Until JavaScript has checked the Bridge's proof, its calls never reach the page.
        for _ in 0..20 {
            let out = ext_core::link_pump();
            if !out.is_empty() {
                ws.send(Message::Binary(out.into()))
                    .await
                    .expect("the Bridge takes the frame");
            }
            if let Ok(Some(Ok(Message::Binary(bytes)))) =
                tokio::time::timeout(Duration::from_millis(25), ws.next()).await
            {
                ext_core::link_feed(&bytes);
            }
            let early = ext_core::link_events();
            assert!(
                !early
                    .iter()
                    .any(|event| matches!(event, LinkEvent::PageCall { .. })),
                "a page call surfaced before link_trust: {early:?}"
            );
        }
        ext_core::link_trust();
        let call = loop {
            let events = next_events(&mut ws).await;
            if let Some(call) = events
                .into_iter()
                .find(|event| matches!(event, LinkEvent::PageCall { .. }))
            {
                break call;
            }
        };
        let LinkEvent::PageCall {
            call_id,
            kind,
            action,
            ..
        } = call
        else {
            unreachable!("matched above")
        };
        assert_eq!(
            (kind.as_str(), action.as_str()),
            ("invoke", "page.getPageInfo")
        );
        ext_core::link_reply(
            call_id,
            json!({"ok": true, "data": {"title": "Smoke"}}).to_string(),
        );
        let out = ext_core::link_pump();
        ws.send(Message::Binary(out.into()))
            .await
            .expect("the answer is sent");
        let data = tokio::time::timeout(Duration::from_secs(10), invoked)
            .await
            .expect("in time")
            .expect("the task");
        assert_eq!(
            serde_json::from_str::<Value>(&data).expect("JSON"),
            json!({"title": "Smoke"})
        );

        // A session request is answered as a TS ActionResult.
        let number = ext_core::link_request(ExtensionRequest::Ping);
        let reply = loop {
            let events = next_events(&mut ws).await;
            if let Some(reply) = events
                .into_iter()
                .find(|event| matches!(event, LinkEvent::Reply { .. }))
            {
                break reply;
            }
        };
        assert_eq!(
            reply,
            LinkEvent::Reply {
                req: number,
                result: r#"{"ok":true,"data":null}"#.into()
            }
        );
    });
}
