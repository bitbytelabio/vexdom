//! A side-panel instruction end to end: the fake extension pairs and sends `instruct`, a stand-in
//! Claude Code replays a recorded stream, and the run's events reach the extension's side panel.
#![allow(clippy::unwrap_used)]

mod support;

#[cfg(unix)]
mod unix {
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use serde_json::json;
    use vexdom_core::{config, lockfile, paths};
    use vexdom_daemon::Daemon;

    use super::support;
    use super::support::fake_extension::{self, Canned, Credential, Outcome};

    /// A stand-in Claude Code: it answers the version probe, reads its turn, then replays Claude
    /// Code 2.1.278 starting a fresh session and answering "ok".
    fn stand_in(home: &Path) -> String {
        use std::os::unix::fs::PermissionsExt;

        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude/2.1.278-fresh.jsonl");
        let bin = home.join("claude");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then echo '2.1.278 (Claude Code)'; exit 0; fi\ncat > /dev/null\ncat '{}'\n",
                fixture.display()
            ),
        )
        .expect("the stand-in can be written");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("the stand-in can be made executable");
        bin.to_string_lossy().into_owned()
    }

    #[test]
    fn an_instruction_runs_the_agent_and_its_events_reach_the_side_panel() {
        let scratch = support::scratch("agent-run");
        std::fs::create_dir_all(&scratch.home).expect("the scratch home can be made");
        let bin = stand_in(&scratch.home);
        let config = config::config_path();
        std::fs::create_dir_all(
            config
                .parent()
                .expect("config.json sits in the state folder"),
        )
        .expect("the state folder can be made");
        std::fs::write(
            &config,
            json!({"agent": "claude", "agents": {"claude": {"bin": bin}}}).to_string(),
        )
        .expect("config.json can be written");

        support::local(async {
            let daemon = Daemon::start(false).await.expect("the Bridge starts");
            let lock = lockfile::read(&paths::lockfile()).expect("the daemon wrote its lockfile");
            let control = vexdom_link::connect_control(lock.port, &lock.token)
                .await
                .expect("the control socket connects")
                .0;
            let paired = control
                .pair_request()
                .send()
                .promise
                .await
                .expect("the Bridge hands out a pairing code");
            let code = paired
                .get()
                .and_then(|reply| reply.get_code())
                .and_then(|code| Ok(code.to_string()?))
                .expect("the reply holds a code");
            let canned: fake_extension::Shared = Rc::new(RefCell::new(Canned::default()));
            let Outcome::Welcome { session, .. } =
                fake_extension::connect(daemon.port(), Credential::Code(&code), Rc::clone(&canned))
                    .await
                    .expect("the fake extension connects")
            else {
                panic!("the pairing code was refused");
            };

            let mut instruct = session.instruct_request();
            instruct.get().set_id("run-1");
            instruct.get().set_text("what does this page say?");
            instruct.get().set_context(
                json!({"sessionId": "conversation-1", "url": "https://example.com/"}).to_string(),
            );
            instruct
                .send()
                .promise
                .await
                .expect("the Bridge takes the instruction");

            let run = || -> Vec<String> {
                canned
                    .borrow()
                    .pushes
                    .iter()
                    .filter_map(|push| push.strip_prefix("run run-1 "))
                    .map(str::to_owned)
                    .collect()
            };
            let deadline = Instant::now() + Duration::from_secs(20);
            while !run()
                .iter()
                .any(|event| event.starts_with("done ") || event.starts_with("error "))
            {
                assert!(
                    Instant::now() < deadline,
                    "the run never ended: {:?}",
                    run()
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let events = run();
            assert!(
                events
                    .first()
                    .is_some_and(|event| event.starts_with("started ")),
                "the run starts by naming its skill: {events:?}"
            );
            assert!(
                events.iter().any(|event| event == "text ok"),
                "the agent's words reach the panel: {events:?}"
            );
            assert_eq!(
                events.last().map(String::as_str),
                Some("done end_turn"),
                "{events:?}"
            );
            daemon.stop();
        });
    }
}
