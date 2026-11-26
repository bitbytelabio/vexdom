//! What the file analyst hands a run about one attached file (`src/lib/files/report.ts`).

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use vexdom_core::agents::AGENT_KINDS;

use vexdom_pure::scrub::{clip, scrub, scrub_lines};

/// `text`, `pdf` or `image`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Text,
    Pdf,
    Image,
}

impl FileKind {
    fn parse(value: Option<&Value>) -> Option<Self> {
        match value?.as_str()? {
            "text" => Some(Self::Text),
            "pdf" => Some(Self::Pdf),
            "image" => Some(Self::Image),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Pdf => "pdf",
            Self::Image => "image",
        }
    }
}

const MB: u64 = 1024 * 1024;

/// Above this the browser keeps only the file's name and size, and the analyst rejects it unread.
pub const MAX_STORED_FILE_BYTES: u64 = 10 * MB;

pub fn file_limit(kind: FileKind) -> u64 {
    match kind {
        FileKind::Text | FileKind::Image => 5 * MB,
        FileKind::Pdf => 10 * MB,
    }
}

pub struct ReportLimits {
    pub summary: usize,
    pub outline: usize,
    pub outline_line: usize,
    pub facts: usize,
    pub fact: usize,
    pub notes: usize,
    pub omitted: usize,
    pub reason: usize,
}

pub const REPORT_LIMITS: ReportLimits = ReportLimits {
    summary: 500,
    outline: 20,
    outline_line: 160,
    facts: 30,
    fact: 200,
    notes: 4_000,
    omitted: 300,
    reason: 300,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reason {
    pub code: String,
    pub message: String,
}

/// `FileReport`, its fields in the TypeScript literal's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileReport {
    /// `analyzed`, `rejected` (not a file Vexdom reads) or `failed` (the analyst broke).
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<FileKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omitted: Option<String>,
}

impl FileReport {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

static REASON_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z][A-Z_]{1,39}$").expect("reason code compiles"));

/// One line, cut with an ellipsis rather than mid-word.
fn line(value: Option<&Value>, limit: usize) -> String {
    clip(&scrub(value, limit * 2), limit)
}

fn reason_of(value: Option<&Value>) -> Option<Reason> {
    let raw = value?;
    let code = raw
        .get("code")
        .and_then(Value::as_str)
        .map(|code| vexdom_pure::text::js_trim(code).to_owned())
        .unwrap_or_default();
    let message = line(raw.get("message"), REPORT_LIMITS.reason);
    (REASON_CODE.is_match(&code) && !message.is_empty()).then_some(Reason { code, message })
}

fn list_of(value: Option<&Value>, count: usize, length: usize) -> Option<Vec<String>> {
    let items: Vec<String> = value?
        .as_array()?
        .iter()
        .take(count)
        .map(|item| line(Some(item), length))
        .filter(|item| !item.is_empty())
        .collect();
    (!items.is_empty()).then_some(items)
}

/// `validateFileReport`.
pub fn validate_file_report(input: &Value) -> Result<FileReport, String> {
    if !input.is_object() {
        return Err("The report is not an object.".into());
    }
    let verdict = match input.get("verdict").and_then(Value::as_str) {
        Some(verdict @ ("analyzed" | "rejected" | "failed")) => verdict.to_owned(),
        _ => return Err("The report has no verdict.".into()),
    };
    let agent = input
        .get("agent")
        .and_then(Value::as_str)
        .filter(|agent| AGENT_KINDS.contains(agent))
        .map(str::to_owned);
    let kind = FileKind::parse(input.get("kind"));
    if verdict != "analyzed" {
        let Some(reason) = reason_of(input.get("reason")) else {
            return Err("A report that did not read the file has to say why.".into());
        };
        return Ok(FileReport {
            verdict,
            agent,
            kind,
            reason: Some(reason),
            ..empty()
        });
    }
    let summary = line(input.get("summary"), REPORT_LIMITS.summary);
    if summary.is_empty() {
        return Err("The report has no summary.".into());
    }
    let partial = input.get("coverage").and_then(Value::as_str) == Some("partial");
    let notes = scrub_lines(input.get("notes"), REPORT_LIMITS.notes);
    let omitted = line(input.get("omitted"), REPORT_LIMITS.omitted);
    Ok(FileReport {
        verdict,
        agent,
        kind,
        reason: None,
        summary: Some(summary),
        outline: list_of(
            input.get("outline"),
            REPORT_LIMITS.outline,
            REPORT_LIMITS.outline_line,
        ),
        facts: list_of(input.get("facts"), REPORT_LIMITS.facts, REPORT_LIMITS.fact),
        notes: (!notes.is_empty()).then_some(notes),
        coverage: Some(if partial { "partial" } else { "full" }.into()),
        omitted: (partial && !omitted.is_empty()).then_some(omitted),
    })
}

fn empty() -> FileReport {
    FileReport {
        verdict: String::new(),
        agent: None,
        kind: None,
        reason: None,
        summary: None,
        outline: None,
        facts: None,
        notes: None,
        coverage: None,
        omitted: None,
    }
}

/// A report on a file that was not read: `rejected` or `failed`.
pub fn refusal(
    verdict: &str,
    agent: Option<&str>,
    code: &str,
    message: &str,
    kind: Option<FileKind>,
) -> FileReport {
    FileReport {
        verdict: verdict.to_owned(),
        agent: agent.map(str::to_owned),
        kind,
        reason: Some(Reason {
            code: code.to_owned(),
            message: line(Some(&json!(message)), REPORT_LIMITS.reason),
        }),
        ..empty()
    }
}

/// "analyzed", "rejected (unsupported type)", "failed (timed out)".
pub fn verdict_label(verdict: &str, code: Option<&str>) -> String {
    if verdict == "analyzed" {
        return verdict.to_owned();
    }
    let label = match code {
        Some("UNSUPPORTED_TYPE") => "unsupported type",
        Some("FILE_TOO_LARGE") => "too large",
        Some("EMPTY") => "empty",
        Some("UNREADABLE") => "unreadable",
        Some("TIMEOUT") => "timed out",
        Some("CANCELLED") => "cancelled",
        _ if verdict == "failed" => "analysis failed",
        _ => "not read",
    };
    format!("{verdict} ({label})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_analyzed_report_is_cleaned_and_bounded() {
        let report = validate_file_report(&json!({
            "verdict": "analyzed", "agent": "codex", "kind": "image",
            "summary": "# A sheet\nof expenses", "outline": ["", "date"], "facts": "not a list",
            "notes": "a  \n\n\n\nb", "coverage": "partial", "omitted": "the rest"
        }))
        .unwrap();
        assert_eq!(
            report.to_json(),
            json!({"verdict": "analyzed", "kind": "image", "summary": "A sheet of expenses",
                   "outline": ["date"], "notes": "a\n\nb", "coverage": "partial", "omitted": "the rest"})
        );
    }

    #[test]
    fn a_report_that_did_not_read_says_why() {
        assert_eq!(
            validate_file_report(
                &json!({"verdict": "failed", "reason": {"code": "x", "message": "m"}})
            ),
            Err("A report that did not read the file has to say why.".into())
        );
        assert_eq!(
            validate_file_report(&json!({"verdict": "analyzed"})),
            Err("The report has no summary.".into())
        );
        assert_eq!(
            refusal("rejected", Some("claude"), "EMPTY", "“a” is empty.", None).to_json(),
            json!({"verdict": "rejected", "agent": "claude", "reason": {"code": "EMPTY", "message": "“a” is empty."}})
        );
    }

    #[test]
    fn verdicts_read_as_one_line() {
        assert_eq!(verdict_label("analyzed", None), "analyzed");
        assert_eq!(
            verdict_label("rejected", Some("UNSUPPORTED_TYPE")),
            "rejected (unsupported type)"
        );
        assert_eq!(
            verdict_label("failed", Some("AGENT_FAILED")),
            "failed (analysis failed)"
        );
        assert_eq!(verdict_label("rejected", None), "rejected (not read)");
    }
}
