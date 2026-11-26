//! A Rust fake extension and a scratch Bridge for the daemon's tests.

#[allow(dead_code)]
pub mod fake_extension;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static ENV: Mutex<()> = Mutex::new(());

/// A fresh `VEXDOM_HOME` and `VEXDOM_PORTS=0` for one test; tests that start a Bridge serialize.
pub struct Scratch {
    #[allow(dead_code)]
    pub home: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

#[allow(unsafe_code)]
pub fn scratch(name: &str) -> Scratch {
    let guard = ENV.lock().unwrap_or_else(|poison| poison.into_inner());
    let home = std::env::temp_dir().join(format!("vexdom-daemon-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    // SAFETY: every test touching the environment holds ENV.
    unsafe {
        std::env::set_var("VEXDOM_HOME", &home);
        std::env::set_var("VEXDOM_PORTS", "0");
        std::env::set_var("HOME", &home);
    }
    Scratch {
        home,
        _guard: guard,
    }
}

/// Runs a test body on a current-thread runtime inside a `LocalSet`.
pub fn local<F: std::future::Future<Output = ()>>(body: F) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, body);
}
