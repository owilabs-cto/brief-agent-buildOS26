use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{fs, path::Path};

pub fn load_fixtures(dir: &Path) -> Result<Vec<Value>> {
    if !dir.exists() {
        fs::create_dir_all(dir).with_context(|| format!("create fixtures dir {dir:?}"))?;
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read_dir {dir:?}"))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {path:?}"))?;
        let v: Value = serde_json::from_str(&raw).with_context(|| format!("parse {path:?}"))?;
        out.push(v);
    }
    Ok(out)
}

pub fn matches(thread: &Value, vc_name: &str) -> bool {
    let needle = vc_name.to_lowercase();

    if str_contains(thread.get("subject"), &needle) {
        return true;
    }
    if array_any_contains(thread.get("labels"), &needle) {
        return true;
    }
    if array_any_contains(thread.get("participants"), &needle) {
        return true;
    }
    if let Some(msgs) = thread.get("messages").and_then(Value::as_array) {
        for m in msgs {
            if str_contains(m.get("body_verbatim"), &needle) {
                return true;
            }
        }
    }
    false
}

pub fn recency_flag(thread: &Value, now: DateTime<Utc>) -> &'static str {
    let Some(s) = thread.get("last_message_date").and_then(Value::as_str) else {
        return "historical";
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(s) else {
        return "historical";
    };
    let age_days = (now - parsed.with_timezone(&Utc)).num_days();
    if age_days > 90 { "historical" } else { "fresh" }
}

fn str_contains(v: Option<&Value>, needle_lower: &str) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| s.to_lowercase().contains(needle_lower))
}

fn array_any_contains(v: Option<&Value>, needle_lower: &str) -> bool {
    v.and_then(Value::as_array).is_some_and(|arr| {
        arr.iter()
            .any(|item| item.as_str().is_some_and(|s| s.to_lowercase().contains(needle_lower)))
    })
}
