//! `vexdom __complete` reads local names (saved states, scheduled tasks) from `VEXDOM_HOME`.

use std::path::Path;
use std::process::Command;

fn complete(home: &Path, words: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_vexdom"))
        .arg("__complete")
        .args(words)
        .env("VEXDOM_HOME", home)
        .output()
        .expect("vexdom runs");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("UTF-8 output")
}

#[test]
fn completes_local_names() {
    let home = std::env::temp_dir().join(format!("vexdom-completions-{}", std::process::id()));
    std::fs::create_dir_all(home.join("states")).unwrap();
    std::fs::write(home.join("states/work.json"), "{}").unwrap();
    std::fs::write(
        home.join("schedules.json"),
        r#"{"paused":false,"tasks":[{"id":"t-abc","name":"Daily","url":"https://example.com","rule":{},"job":{}}]}"#,
    )
    .unwrap();

    let states = complete(&home, &["2", "state", "load", ""]);
    let tasks = complete(&home, &["2", "tasks", "pause", ""]);
    std::fs::remove_dir_all(&home).unwrap();

    assert_eq!(states, ":words\nwork\n");
    assert_eq!(tasks, ":words\nt-abc\tDaily\n");
}
