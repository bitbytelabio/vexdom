//! `ActionResult` (`src/lib/actions/protocol.ts`): `{ok:true,data}` or `{ok:false,error:{code,message}}`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ActionError {}

pub type ActionResult = Result<Value, ActionError>;

pub fn failure<T>(code: &str, message: impl Into<String>) -> Result<T, ActionError> {
    Err(ActionError {
        code: code.to_owned(),
        message: message.into(),
    })
}

impl From<(String, String)> for ActionError {
    fn from((code, message): (String, String)) -> Self {
        Self { code, message }
    }
}

/// The TS JSON shape, for documents that carry a result (`agentInfo`, file reports, …).
pub fn to_json(result: &ActionResult) -> Value {
    match result {
        Ok(data) => json!({"ok": true, "data": data}),
        Err(error) => json!({"ok": false, "error": {"code": error.code, "message": error.message}}),
    }
}

/// Reads the TS JSON shape back.
pub fn from_json(value: &Value) -> ActionResult {
    if value.get("ok") == Some(&Value::Bool(true)) {
        return Ok(value.get("data").cloned().unwrap_or(Value::Null));
    }
    let error = value.get("error").cloned().unwrap_or(Value::Null);
    Err(ActionError {
        code: error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_owned(),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    })
}
