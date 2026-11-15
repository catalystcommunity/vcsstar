//! Repository sync across independent remote connections.

pub mod engine;
pub mod local;
pub mod remote;
pub mod service;
pub mod store;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Provider-neutral content. Attribution and original time are preserved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub kind: String,
    pub data: Value,
    pub actor: Option<Uuid>,
    pub author: String,
    pub created_at: i64,
    pub url: Option<String>,
}

impl Item {
    pub fn equivalent(&self, other: &Self) -> bool {
        self.kind == other.kind && self.data == other.data
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Equal,
    Import,
    Export,
    Conflict,
}

/// Compare each side with its own last acknowledged representation. A
/// summary on a remote can differ from the full local record indefinitely.
pub fn decide(
    local: Option<&Item>,
    remote: Option<&Item>,
    local_base: Option<&Item>,
    remote_base: Option<&Item>,
) -> Decision {
    fn same(a: Option<&Item>, b: Option<&Item>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => a.equivalent(b),
            (None, None) => true,
            _ => false,
        }
    }
    if same(local, remote) {
        return Decision::Equal;
    }
    match (!same(local, local_base), !same(remote, remote_base)) {
        (false, false) => Decision::Equal,
        (false, true) => Decision::Import,
        (true, false) => Decision::Export,
        (true, true) => Decision::Conflict,
    }
}

pub fn digest(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.to_string().as_bytes()))
}

pub fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
pub fn encoded(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(kind: &str, value: &str) -> Item {
        Item {
            kind: kind.into(),
            data: serde_json::json!({"body":value}),
            actor: None,
            author: "author".into(),
            created_at: 0,
            url: None,
        }
    }
    #[test]
    fn reconciliation_tracks_both_acknowledged_projections() {
        let full = item("review", "full");
        let summary = item("comment", "summary");
        let local_edit = item("review", "edited");
        let remote_edit = item("comment", "edited");
        assert_eq!(
            decide(Some(&full), Some(&summary), Some(&full), Some(&summary)),
            Decision::Equal
        );
        assert_eq!(
            decide(
                Some(&local_edit),
                Some(&summary),
                Some(&full),
                Some(&summary)
            ),
            Decision::Export
        );
        assert_eq!(
            decide(Some(&full), Some(&remote_edit), Some(&full), Some(&summary)),
            Decision::Import
        );
        assert_eq!(
            decide(
                Some(&local_edit),
                Some(&remote_edit),
                Some(&full),
                Some(&summary)
            ),
            Decision::Conflict
        );
        assert_eq!(
            decide(None, Some(&summary), Some(&full), Some(&summary)),
            Decision::Export
        );
        assert_eq!(
            decide(Some(&full), None, Some(&full), Some(&summary)),
            Decision::Import
        );
        assert_eq!(
            decide(None, Some(&remote_edit), Some(&full), Some(&summary)),
            Decision::Conflict
        );
        assert_eq!(
            decide(Some(&local_edit), None, Some(&full), Some(&summary)),
            Decision::Conflict
        );
        assert_eq!(decide(Some(&full), None, None, None), Decision::Export);
        assert_eq!(decide(None, Some(&full), None, None), Decision::Import);
        assert_eq!(
            decide(Some(&full), Some(&local_edit), None, None),
            Decision::Conflict
        );
        assert_eq!(
            decide(None, None, Some(&full), Some(&summary)),
            Decision::Equal
        );
        // An intentionally omitted destination stays acknowledged.
        assert_eq!(
            decide(Some(&full), None, Some(&full), None),
            Decision::Equal
        );
        assert_eq!(
            decide(
                Some(&local_edit),
                Some(&local_edit),
                Some(&full),
                Some(&full)
            ),
            Decision::Equal
        );
    }
    #[test]
    fn destination_edits_preserve_fields_missing_from_a_projection() {
        let mut full = item("comment", "original");
        full.data = serde_json::json!({"body":"original","path":"src/main.rs","line":7,"commit_sha":"abc","parent":"pr:one"});
        let mut baseline = full.clone();
        baseline.data = serde_json::json!({"body":"src/main.rs:7\n\noriginal","path":null,"line":null,"commit_sha":null,"parent":"pr:one"});
        let mut changed = baseline.clone();
        changed.data["body"] = serde_json::json!("src/main.rs:7\n\nedited remotely");
        let imported = remote::import_projection(&full, &changed, Some(&baseline)).unwrap();
        assert_eq!(imported.data["body"], "edited remotely");
        assert_eq!(imported.data["path"], "src/main.rs");
        assert_eq!(imported.data["line"], 7);
        assert_eq!(imported.data["commit_sha"], "abc");
        // Closing a destination copy does not erase an acknowledged merge.
        let mut merged = item("pr", "body");
        merged.data["state"] = serde_json::json!("merged");
        let mut closed = merged.clone();
        closed.data["state"] = serde_json::json!("closed");
        let mut edited = closed.clone();
        edited.data["body"] = serde_json::json!("new body");
        let imported = remote::import_projection(&merged, &edited, Some(&closed)).unwrap();
        assert_eq!(imported.data["state"], "merged");
        assert_eq!(imported.data["body"], "new body");
    }
}
