//! Pure logic shared by the Bridge and ext-core (wasm): no I/O, clock, randomness or system time
//! zone. Each module names the TypeScript it mirrors.

pub mod blocked_sites;
pub mod handshake;
pub mod intent;
pub mod profile;
pub mod schedules;
pub mod scrub;
pub mod seal;
pub mod secrets;
pub mod text;
pub mod workflow;
