//! Notices a browser hands to the desktop app (`src/lib/notices.ts`, `src/daemon/notice-desk.ts`).
//! Held in memory only: a notice is worth showing for minutes, not more.

use serde::Serialize;
use serde_json::Value;

use vexdom_pure::text::utf16_prefix;

const KEEP_MS: i64 = 5 * 60_000;
const QUEUE_MAX: usize = 20;
const ROUTES_MAX: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceNotice {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub tone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueuedNotice {
    #[serde(flatten)]
    pub notice: DeviceNotice,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    pub at: i64,
}

/// `asDeviceNotice`: validates and clamps (title 120, body 400).
pub fn as_device_notice(value: &Value) -> Option<DeviceNotice> {
    let id = value.get("id")?.as_str()?;
    if id.is_empty() || id.encode_utf16().count() > 120 {
        return None;
    }
    let title = value.get("title")?.as_str()?;
    let body = value.get("body")?.as_str()?;
    let kind = value.get("kind")?.as_str()?;
    if !["monitor", "task", "timer"].contains(&kind) {
        return None;
    }
    Some(DeviceNotice {
        id: id.to_owned(),
        kind: kind.to_owned(),
        title: utf16_prefix(title, 120).to_owned(),
        body: utf16_prefix(body, 400).to_owned(),
        tone: if value.get("tone").and_then(Value::as_str) == Some("warn") {
            "warn".into()
        } else {
            "live".into()
        },
    })
}

/// Holds notices until the app takes them. Watchers are identified by an id the daemon chooses.
#[derive(Default)]
pub struct NoticeDesk {
    allowed_by: Vec<(u64, bool)>,
    queue: Vec<QueuedNotice>,
    routes: Vec<(String, String)>,
}

impl NoticeDesk {
    pub fn watch(&mut self, watcher: u64, allowed: bool) {
        match self.allowed_by.iter_mut().find(|(id, _)| *id == watcher) {
            Some(entry) => entry.1 = allowed,
            None => self.allowed_by.push((watcher, allowed)),
        }
    }

    pub fn unwatch(&mut self, watcher: u64) {
        self.allowed_by.retain(|(id, _)| *id != watcher);
    }

    pub fn can_show(&self) -> bool {
        self.allowed_by.iter().any(|(_, allowed)| *allowed)
    }

    /// The watchers that may show notices.
    pub fn showers(&self) -> Vec<u64> {
        self.allowed_by
            .iter()
            .filter(|(_, allowed)| *allowed)
            .map(|(id, _)| *id)
            .collect()
    }

    /// True when an app will show it; the caller then tells the showers.
    pub fn offer(
        &mut self,
        notice: DeviceNotice,
        from_id: &str,
        browser: Option<String>,
        now: i64,
    ) -> bool {
        if !self.can_show() {
            return false;
        }
        self.queue.retain(|queued| queued.notice.id != notice.id);
        let id = notice.id.clone();
        self.queue.push(QueuedNotice {
            notice,
            browser,
            at: now,
        });
        if self.queue.len() > QUEUE_MAX {
            let cut = self.queue.len() - QUEUE_MAX;
            self.queue.drain(..cut);
        }
        self.routes.retain(|(notice, _)| *notice != id);
        self.routes.push((id, from_id.to_owned()));
        if self.routes.len() > ROUTES_MAX {
            let cut = self.routes.len() - ROUTES_MAX;
            self.routes.drain(..cut);
        }
        true
    }

    /// What is waiting, for a watcher that may show it.
    pub fn take(&mut self, watcher: u64, now: i64) -> Vec<QueuedNotice> {
        if !self
            .allowed_by
            .iter()
            .any(|(id, allowed)| *id == watcher && *allowed)
        {
            return Vec::new();
        }
        std::mem::take(&mut self.queue)
            .into_iter()
            .filter(|queued| now - queued.at < KEEP_MS)
            .collect()
    }

    pub fn route_of(&self, notice_id: &str) -> Option<&str> {
        self.routes
            .iter()
            .find(|(notice, _)| notice == notice_id)
            .map(|(_, route)| route.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn desk() {
        let notice = as_device_notice(&json!({"id": "n1", "kind": "task", "title": "t".repeat(200), "body": "b", "tone": "x"})).unwrap();
        assert_eq!((notice.title.len(), notice.tone.as_str()), (120, "live"));
        assert!(
            as_device_notice(&json!({"id": "n1", "kind": "other", "title": "", "body": ""}))
                .is_none()
        );
        let mut desk = NoticeDesk::default();
        assert!(!desk.offer(notice.clone(), "b1", None, 0));
        desk.watch(7, true);
        assert!(desk.offer(notice.clone(), "b1", Some("Chrome".into()), 0));
        assert_eq!(desk.route_of("n1"), Some("b1"));
        assert!(desk.take(8, 1).is_empty());
        let taken = desk.take(7, 1);
        assert_eq!(taken.len(), 1);
        assert_eq!(
            serde_json::to_value(&taken[0]).unwrap()["browser"],
            "Chrome"
        );
        assert!(desk.take(7, 1).is_empty());
    }
}
