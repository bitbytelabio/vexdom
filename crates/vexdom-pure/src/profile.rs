//! The user profile's shape (`src/lib/settings/profile.ts`): what the settings page may hold, and
//! its normal form. The file it lives in is `vexdom_core::profile`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::text::js_trim;

/// `PROFILE_FIELDS`: id and label, in form order.
pub const PROFILE_FIELDS: [(&str, &str); 11] = [
    ("givenName", "First name"),
    ("familyName", "Last name"),
    ("email", "Email"),
    ("phone", "Phone"),
    ("street", "Street address"),
    ("city", "City"),
    ("region", "State / region"),
    ("postalCode", "Postal code"),
    ("country", "Country"),
    ("company", "Company"),
    ("jobTitle", "Job title"),
];

const VALUE_MAX: usize = 300;
const LABEL_MAX: usize = 60;
const DETAILS_MAX: usize = 20;
const INSTRUCTIONS_MAX: usize = 4000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDetail {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    /// Field id → text, in `PROFILE_FIELDS` order once normalized.
    pub fields: Map<String, Value>,
    pub details: Vec<ProfileDetail>,
    pub instructions: String,
}

fn fits(value: &Value, limit: usize) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.encode_utf16().count() <= limit)
}

pub fn is_field_id(id: &str) -> bool {
    PROFILE_FIELDS.iter().any(|(field, _)| *field == id)
}

fn is_detail(value: &Value) -> bool {
    value.is_object()
        && value
            .get("label")
            .is_some_and(|label| fits(label, LABEL_MAX))
        && value.get("value").is_some_and(|text| fits(text, VALUE_MAX))
}

/// `isUserProfile`: the exact shape the settings page writes.
pub fn is_user_profile(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let fields_ok = object
        .get("fields")
        .and_then(Value::as_object)
        .is_some_and(|fields| {
            fields
                .iter()
                .all(|(id, text)| is_field_id(id) && fits(text, VALUE_MAX))
        });
    let details_ok = object
        .get("details")
        .and_then(Value::as_array)
        .is_some_and(|details| details.len() <= DETAILS_MAX && details.iter().all(is_detail));
    fields_ok
        && details_ok
        && object
            .get("instructions")
            .is_some_and(|text| fits(text, INSTRUCTIONS_MAX))
}

impl UserProfile {
    pub fn normalized(&self) -> Self {
        let fields = PROFILE_FIELDS
            .iter()
            .filter_map(|(id, _)| {
                let text = js_trim(self.fields.get(*id)?.as_str()?);
                (!text.is_empty()).then(|| ((*id).to_owned(), Value::String(text.to_owned())))
            })
            .collect();
        let details = self
            .details
            .iter()
            .map(|detail| ProfileDetail {
                label: js_trim(&detail.label).to_owned(),
                value: js_trim(&detail.value).to_owned(),
            })
            .filter(|detail| !detail.label.is_empty() && !detail.value.is_empty())
            .collect();
        Self {
            fields,
            details,
            instructions: js_trim(&self.instructions).to_owned(),
        }
    }

    pub fn is_empty(&self) -> bool {
        let normal = self.normalized();
        normal.fields.is_empty() && normal.details.is_empty() && normal.instructions.is_empty()
    }

    pub fn field(&self, id: &str) -> &str {
        self.fields.get(id).and_then(Value::as_str).unwrap_or("")
    }
}

/// `asUserProfile`: drops what does not fit rather than cutting it short.
pub fn as_user_profile(value: &Value) -> UserProfile {
    let Some(object) = value.as_object() else {
        return UserProfile::default();
    };
    let fields = object
        .get("fields")
        .and_then(Value::as_object)
        .map(|fields| {
            fields
                .iter()
                .filter(|(id, text)| is_field_id(id) && fits(text, VALUE_MAX))
                .map(|(id, text)| (id.clone(), text.clone()))
                .collect()
        })
        .unwrap_or_default();
    let details = object
        .get("details")
        .and_then(Value::as_array)
        .map(|details| {
            details
                .iter()
                .filter(|detail| is_detail(detail))
                .take(DETAILS_MAX)
                .filter_map(|detail| serde_json::from_value(detail.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let instructions = object
        .get("instructions")
        .filter(|text| fits(text, INSTRUCTIONS_MAX))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    UserProfile {
        fields,
        details,
        instructions,
    }
    .normalized()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes() {
        let value = json!({"fields": {"email": " a@b.c ", "givenName": "Ada", "bogus": "x"},
                           "details": [{"label": " Size ", "value": "M"}, {"label": "", "value": "x"}],
                           "instructions": " be brief "});
        assert!(!is_user_profile(&value));
        let profile = as_user_profile(&value);
        assert_eq!(
            serde_json::to_value(&profile).unwrap(),
            json!({"fields": {"givenName": "Ada", "email": "a@b.c"}, "details": [{"label": "Size", "value": "M"}], "instructions": "be brief"})
        );
        assert!(!profile.is_empty());
        assert!(UserProfile::default().is_empty());
    }
}
