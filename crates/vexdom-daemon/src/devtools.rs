//! `page.vitals`, `page.react` and `page.a11yAudit` on the Bridge's side: the reports agent-browser
//! prints, made from the raw JSON the extension's page scripts answer; and axe-core fetched once
//! from the npm registry into the cache (ADR-030), never bundled.

use std::io::Read as _;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha512};
use vexdom_core::paths;
use vexdom_core::result::ActionError;

use crate::link::Reply;
use crate::react;

pub const AXE_VERSION: &str = "4.12.1";
const REGISTRY_TIMEOUT: Duration = Duration::from_secs(15);

/// agent-browser `handle_vitals`: Core Web Vitals and React hydration phases from the raw read.
pub fn vitals_report(url: &str, raw: &Value) -> Value {
    let cwv = raw.get("cwv").cloned().unwrap_or(json!({}));
    let timing = raw
        .get("timing")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let ttfb = raw.get("ttfb").and_then(|v| v.as_f64());
    let lcp = cwv.get("lcp").cloned().unwrap_or(Value::Null);
    let cls_score = cwv.get("cls").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cls_entries = cwv.get("clsEntries").cloned().unwrap_or(json!([]));
    let fcp = cwv.get("fcp").and_then(|v| v.as_f64());
    let inp = cwv.get("inp").and_then(|v| v.as_f64());

    let round = |n: f64| (n * 100.0).round() / 100.0;

    let mut hydration_phases: Vec<Value> = Vec::new();
    let mut hydration_start = f64::INFINITY;
    let mut hydration_end = 0.0f64;
    let mut hydrated_components: Vec<Value> = Vec::new();
    // React's profiling build emits `console.timeStamp(label, start, end,
    // track, trackGroup, color)` entries whose `track` / `trackGroup`
    // fields are literal strings containing the atom glyph (e.g.
    // "Scheduler ⚛", "Components ⚛"). The comparisons below match those
    // exact strings — don't "clean up" the glyphs.
    for e in &timing {
        let label = e.get("label").and_then(|v| v.as_str()).unwrap_or("");
        let track = e.get("track").and_then(|v| v.as_str()).unwrap_or("");
        let track_group = e.get("trackGroup").and_then(|v| v.as_str()).unwrap_or("");
        let color = e.get("color").and_then(|v| v.as_str()).unwrap_or("");
        let start = e.get("startTime").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let end = e.get("endTime").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if end <= start {
            continue;
        }
        if track_group == "Scheduler ⚛" {
            hydration_phases.push(json!({
                "label": label,
                "startTime": round(start),
                "endTime": round(end),
                "duration": round(end - start),
            }));
            if label == "Hydrated" {
                if start < hydration_start {
                    hydration_start = start;
                }
                if end > hydration_end {
                    hydration_end = end;
                }
            }
        } else if track == "Components ⚛" && color.starts_with("tertiary") {
            hydrated_components.push(json!({
                "name": label,
                "startTime": round(start),
                "endTime": round(end),
                "duration": round(end - start),
            }));
        }
    }
    hydrated_components.sort_by(|a, b| {
        let da = a.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let db = b.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });

    let hydration = if hydration_start.is_finite() && hydration_end > 0.0 {
        json!({
            "startTime": round(hydration_start),
            "endTime": round(hydration_end),
            "duration": round(hydration_end - hydration_start),
        })
    } else {
        Value::Null
    };

    let data_value = json!({
        "url": url,
        "ttfb": ttfb,
        "lcp": lcp,
        "cls": { "score": round(cls_score), "entries": cls_entries },
        "fcp": fcp,
        "inp": inp,
        "hydration": hydration,
        "phases": hydration_phases,
        "hydratedComponents": hydrated_components,
    });
    data_value
}

/// The reply of `page.react` as agent-browser prints it: a text report unless `json` is asked.
fn react_report(input: &Value, data: &Value) -> Result<Value, String> {
    let json = input.get("json") == Some(&Value::Bool(true));
    let Some(raw) = data.get("raw") else {
        return Ok(data.clone());
    };
    match data.get("op").and_then(Value::as_str) {
        Some("tree") => {
            let nodes: Vec<react::TreeNode> = serde_json::from_value(raw.clone())
                .map_err(|e| format!("Failed to parse tree nodes: {e}"))?;
            if json {
                Ok(
                    json!({"nodes": nodes.iter().map(|n| json!({"id": n.id, "type": n.node_type, "name": n.name, "key": n.key, "parent": n.parent})).collect::<Vec<_>>()}),
                )
            } else {
                Ok(json!({"tree": react::format_tree(&nodes)}))
            }
        }
        Some("rendersStop") => {
            if json {
                return Ok(raw.clone());
            }
            let data: react::RendersData = serde_json::from_value(raw.clone())
                .map_err(|e| format!("Failed to parse renders data: {e}"))?;
            Ok(json!({"report": react::format_renders_report(&data)}))
        }
        Some("suspense") => {
            let boundaries: Vec<react::Boundary> = serde_json::from_value(raw.clone())
                .map_err(|e| format!("Failed to parse suspense boundaries: {e}"))?;
            let only_dynamic = input.get("onlyDynamic") == Some(&Value::Bool(true));
            if json {
                let kept: Vec<&react::Boundary> = boundaries
                    .iter()
                    .filter(|b| {
                        !only_dynamic
                            || (b.parent_id != 0
                                && (b.is_suspended
                                    || !b.suspended_by.is_empty()
                                    || b.unknown_suspenders.is_some()))
                    })
                    .collect();
                Ok(json!({"boundaries": kept}))
            } else {
                Ok(json!({"report": react::format_suspense_report(&boundaries, only_dynamic)}))
            }
        }
        _ => Ok(raw.clone()),
    }
}

/// Turns the raw answers of `page.vitals` and `page.react` into their reports.
pub fn finish(action: &str, input: &Value, mut reply: Reply) -> Reply {
    let Ok(data) = &reply.result else {
        return reply;
    };
    let shaped = match action {
        "page.vitals" => Ok(vitals_report(
            data.get("url").and_then(Value::as_str).unwrap_or(""),
            data.get("raw").unwrap_or(&Value::Null),
        )),
        "page.react" => react_report(input, data),
        _ => return reply,
    };
    reply.result = shaped.map_err(|message| ActionError {
        code: "ACTION_FAILED".into(),
        message,
    });
    reply
}

fn unavailable(reason: impl std::fmt::Display) -> ActionError {
    ActionError {
        code: "AXE_UNAVAILABLE".into(),
        message: reason.to_string(),
    }
}

fn axe_cache() -> std::path::PathBuf {
    paths::state_dir()
        .join("cache")
        .join(format!("axe-core-{AXE_VERSION}.min.js"))
}

/// axe-core's script: the cached copy, else the npm tarball, checked against the registry's
/// sha512 integrity before its `package/axe.min.js` is kept.
pub async fn axe_source() -> Result<String, ActionError> {
    let cache = axe_cache();
    if let Ok(source) = std::fs::read_to_string(&cache) {
        return Ok(source);
    }
    let client = reqwest::Client::builder()
        .timeout(REGISTRY_TIMEOUT)
        .build()
        .map_err(unavailable)?;
    let meta = client
        .get(format!("https://registry.npmjs.org/axe-core/{AXE_VERSION}"))
        .send()
        .await
        .map_err(unavailable)?
        .error_for_status()
        .map_err(unavailable)?
        .bytes()
        .await
        .map_err(unavailable)?;
    let meta: Value = serde_json::from_slice(&meta).map_err(unavailable)?;
    let dist = meta
        .get("dist")
        .ok_or_else(|| unavailable("the registry gave no dist"))?;
    let tarball = dist
        .get("tarball")
        .and_then(Value::as_str)
        .ok_or_else(|| unavailable("the registry gave no tarball"))?;
    let integrity = dist
        .get("integrity")
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix("sha512-"))
        .ok_or_else(|| unavailable("the registry gave no sha512 integrity"))?;
    let bytes = client
        .get(tarball)
        .send()
        .await
        .map_err(unavailable)?
        .error_for_status()
        .map_err(unavailable)?
        .bytes()
        .await
        .map_err(unavailable)?;
    let digest = base64::engine::general_purpose::STANDARD.encode(Sha512::digest(&bytes));
    if digest != integrity {
        return Err(unavailable(
            "the tarball does not match the registry's integrity hash",
        ));
    }
    let source = axe_from_tarball(&bytes).map_err(unavailable)?;
    if let Some(parent) = cache.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = write_private(&cache, source.as_bytes());
    Ok(source)
}

fn axe_from_tarball(bytes: &[u8]) -> Result<String, String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.to_path_buf();
        if path == std::path::Path::new("package/axe.min.js") {
            let mut source = String::new();
            entry
                .read_to_string(&mut source)
                .map_err(|e| e.to_string())?;
            return Ok(source);
        }
    }
    Err("the tarball holds no package/axe.min.js".into())
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vitals_take_the_hydrated_span_and_sort_components_by_duration() {
        let raw = json!({
            "cwv": {"lcp": {"startTime": 812.5}, "cls": 0.0123, "clsEntries": [], "fcp": 400.0, "inp": null},
            "ttfb": 120.0,
            "timing": [
                {"label": "Hydrated", "trackGroup": "Scheduler ⚛", "startTime": 100.0, "endTime": 180.0},
                {"label": "Nav", "track": "Components ⚛", "color": "tertiary-dark", "startTime": 100.0, "endTime": 110.0},
                {"label": "Page", "track": "Components ⚛", "color": "tertiary", "startTime": 110.0, "endTime": 170.0},
            ],
        });
        let report = vitals_report("https://a.test/", &raw);
        assert_eq!(report["ttfb"], 120.0);
        assert_eq!(report["cls"]["score"], 0.01);
        assert_eq!(report["hydration"]["duration"], 80.0);
        assert_eq!(report["hydratedComponents"][0]["name"], "Page");
    }

    #[test]
    fn a_react_tree_prints_as_text_or_nodes() {
        let data = json!({"op": "tree", "raw": [
            {"id": 1, "type": 11, "name": null, "key": null, "parent": 0},
            {"id": 2, "type": 0, "name": "App", "key": null, "parent": 1},
        ]});
        let text = react_report(&json!({}), &data).unwrap();
        assert!(
            text["tree"]
                .as_str()
                .unwrap()
                .ends_with("0 1 - Root\n1 2 1 App")
        );
        let nodes = react_report(&json!({"json": true}), &data).unwrap();
        assert_eq!(nodes["nodes"][1]["name"], "App");
    }
}
