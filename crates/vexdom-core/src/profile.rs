//! `profile.json` (`src/daemon/profile.ts`). Kept out of config.json so the config stays safe to
//! paste into a bug report. Its shape is `vexdom_pure::profile`.

use std::path::PathBuf;

use serde_json::Value;
use vexdom_pure::profile::{UserProfile, as_user_profile, is_user_profile};

use crate::{log, paths};

pub fn profile_path() -> PathBuf {
    paths::state_dir().join("profile.json")
}

pub fn read_profile_text() -> String {
    std::fs::read_to_string(profile_path()).unwrap_or_default()
}

pub fn read_profile() -> UserProfile {
    let text = read_profile_text();
    if text.is_empty() {
        return UserProfile::default();
    }
    let path = profile_path();
    let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
        log::log(&format!(
            "{} is not JSON; the agent runs without a profile until it is fixed or saved again",
            path.display()
        ));
        return UserProfile::default();
    };
    if !is_user_profile(&parsed) {
        log::log(&format!(
            "{} has entries the settings page cannot hold; they are left out",
            path.display()
        ));
    }
    as_user_profile(&parsed)
}

/// Deleted when empty; otherwise the normalized profile, mode 0600.
pub fn write_profile(profile: &UserProfile) -> std::io::Result<()> {
    if profile.is_empty() {
        let _ = std::fs::remove_file(profile_path());
        return Ok(());
    }
    paths::write_private_json(&profile_path(), &profile.normalized())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_store::tests::scratch_home;
    use serde_json::json;

    #[test]
    fn round_trips() {
        let _home = scratch_home("profile");
        let value = json!({"fields": {"email": " a@b.c ", "givenName": "Ada"},
                           "details": [{"label": " Size ", "value": "M"}], "instructions": " be brief "});
        let profile = as_user_profile(&value);
        write_profile(&profile).unwrap();
        assert_eq!(read_profile(), profile);
        write_profile(&UserProfile::default()).unwrap();
        assert!(!profile_path().exists());
    }
}
