//! Candidate deliverables are working data, separate from verified claims.
//! Keep whole drafts through compaction/repair; publishing one never applies it.
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_ARTIFACT_BYTES: usize = 8 * 1024;
const MAX_TOTAL_BYTES: usize = 12 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DraftArtifactV1 {
    pub title: String,
    pub language: String,
    pub content: String,
    /// Recomputed locally; parsing is not proof of combat equivalence.
    #[serde(default)]
    pub syntax: String,
}

impl DraftArtifactV1 {
    fn normalize(mut self) -> Option<Self> {
        if self.content.trim().is_empty()
            || self.content.len() > MAX_ARTIFACT_BYTES
            || super::session::contains_likely_secret(&self.content)
        {
            return None;
        }
        self.title = super::session::redact_sensitive_text(&self.title)
            .chars()
            .take(120)
            .collect();
        self.language = self.language.chars().take(32).collect();
        self.syntax = if matches!(self.language.as_str(), "jx3_macro" | "macro") {
            self.language = "jx3_macro".into();
            if crate::macro_parser::parse_macro_text(&self.content).is_ok() {
                "parsed"
            } else {
                "invalid"
            }
        } else {
            "unchecked"
        }
        .into();
        Some(self)
    }
}

#[derive(Debug, Default)]
pub struct ArtifactStore {
    items: Vec<DraftArtifactV1>,
}

impl ArtifactStore {
    pub fn extend(&mut self, items: impl IntoIterator<Item = DraftArtifactV1>) {
        for item in items {
            let Some(item) = item.normalize() else {
                continue;
            };
            // Latest revision replaces its named draft; code is never sliced.
            self.items
                .retain(|old| old.title != item.title && old.content != item.content);
            self.items.push(item);
            while self.items.len() > 3
                || self.items.iter().map(|a| a.content.len()).sum::<usize>() > MAX_TOTAL_BYTES
            {
                self.items.remove(0);
            }
        }
    }

    pub fn capture_report(&mut self, raw: &str) {
        let raw = raw.trim();
        let raw = raw
            .strip_prefix("```json")
            .or_else(|| raw.strip_prefix("```"))
            .unwrap_or(raw);
        if let Ok(value) = serde_json::from_str::<Value>(raw.trim().trim_end_matches("```").trim())
        {
            self.capture_value(&value);
        }
    }

    fn capture_value(&mut self, value: &Value) {
        if let Some(items) = value.get("artifacts").and_then(Value::as_array) {
            self.extend(
                items
                    .iter()
                    .filter_map(|item| serde_json::from_value(item.clone()).ok()),
            );
        }
        // Older models used an invented edit type for a complete new macro.
        if let Some(changes) = value.get("rotation_changes").and_then(Value::as_array) {
            for change in changes {
                if matches!(
                    change["change_type"].as_str(),
                    Some("add_macro_page" | "new_macro" | "macro_text")
                ) {
                    if let Some(content) = change["proposed"].as_str() {
                        self.extend([DraftArtifactV1 {
                            title: "完整宏候选".into(),
                            language: "jx3_macro".into(),
                            content: content.into(),
                            syntax: String::new(),
                        }]);
                    }
                }
            }
        }
    }

    pub fn capture_session(&mut self, context: &str) {
        if let Ok(value) = serde_json::from_str::<Value>(context) {
            self.capture_value(&value);
            if let Some(turns) = value["turns"].as_array() {
                for turn in turns {
                    self.capture_value(turn);
                }
            }
        }
    }

    pub fn capture_tool(&mut self, name: &str, arguments: &Value) {
        if name != "compare_scenarios" {
            return;
        }
        if let Some(candidates) = arguments["candidates"].as_array() {
            for candidate in candidates {
                if let Some(content) = candidate
                    .pointer("/patch/macro_text")
                    .and_then(Value::as_str)
                {
                    self.extend([DraftArtifactV1 {
                        title: candidate["label"].as_str().unwrap_or("完整宏候选").into(),
                        language: "jx3_macro".into(),
                        content: content.into(),
                        syntax: String::new(),
                    }]);
                }
            }
        }
    }

    pub fn items(&self) -> Vec<DraftArtifactV1> {
        self.items.clone()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn restore_report(&self, raw: &str) -> String {
        let Ok(mut value) = serde_json::from_str::<Value>(raw.trim()) else {
            return raw.into();
        };
        if !value.is_object() || self.is_empty() {
            return raw.into();
        }
        let mut merged = Self::default();
        merged.capture_value(&value);
        if merged.is_empty() { merged.extend(self.items()); }
        let mut selected = merged.items();
        normalize_artifacts(&mut selected);
        value["artifacts"] = serde_json::to_value(selected).unwrap();
        value.to_string()
    }

    pub fn context_message(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        Some(format!("<working_artifacts source=\"model_drafts\" status=\"candidate\">\n{}\n</working_artifacts>\n以上是已保存的完整候选。围绕用户目标修改或验证这份候选，最终 artifacts 交付采用的完整文本；复刻程度依据实际对照结果说明。", serde_json::to_string(&self.items).unwrap_or_default()))
    }
}

pub fn normalize_artifacts(items: &mut Vec<DraftArtifactV1>) {
    let mut store = ArtifactStore::default();
    store.extend(std::mem::take(items));
    *items = store.items();
    if let Some(last_macro) = items.iter().rposition(|item| item.language == "jx3_macro") {
        let mut index = 0;
        items.retain(|item| {
            let keep = item.language != "jx3_macro" || index == last_macro;
            index += 1;
            keep
        });
    }
}

pub fn json_schema() -> Value {
    serde_json::json!({"type":"array", "maxItems":3, "items":{
        "type":"object", "properties":{
            "title":{"type":"string","maxLength":120},
            "language":{"type":"string","description":"jx3_macro for complete macro candidates; otherwise the code language.","maxLength":32},
            "content":{"type":"string","description":"Complete candidate text, separate from verified claims. Retained through continuation and repair.","maxLength":8192}
        }, "required":["title","language","content"], "additionalProperties":false
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_selection_does_not_republish_earlier_candidates() {
        let mut store = ArtifactStore::default();
        store.capture_report(r#"{"artifacts":[{"title":"旧版","language":"jx3_macro","content":"/cast 盾击"}]}"#);
        let report = store.restore_report(r#"{"artifacts":[{"title":"最终版","language":"jx3_macro","content":"/cast 斩刀"}]}"#);
        let value: Value = serde_json::from_str(&report).unwrap();
        assert_eq!(value["artifacts"].as_array().unwrap().len(), 1);
        assert_eq!(value["artifacts"][0]["title"], "最终版");
        assert_eq!(store.items()[0].title, "旧版");
    }
    #[test]
    fn legacy_macro_survives_empty_final_and_is_not_promoted_to_verified() {
        let mut store = ArtifactStore::default();
        store.capture_report(r##"{"rotation_changes":[{"change_type":"add_macro_page","proposed":"#page shield\n/cast 盾击\n#page blade\n/cast 斩刀"}]}"##);
        store.capture_report(r#"{"summary":"宏文本如下","rotation_changes":[],"artifacts":[]}"#);
        assert_eq!(store.items().len(), 1);
        assert_eq!(store.items()[0].syntax, "parsed");
        assert!(store.context_message().unwrap().contains("/cast 斩刀"));
    }
    #[test]
    fn revisions_replace_whole_code_and_ignore_claimed_status() {
        let mut store = ArtifactStore::default();
        for code in ["/cast 盾击", "/cast [rage>] 绝刀"] {
            store.capture_report(&serde_json::json!({"artifacts":[{"title":"候选", "language":"jx3_macro", "content":code,"syntax":"verified"}]}).to_string());
        }
        assert_eq!(store.items().len(), 1);
        assert_eq!(store.items()[0].content, "/cast [rage>] 绝刀");
        assert_eq!(store.items()[0].syntax, "invalid");
    }

    #[test]
    fn windows_newlines_survive_and_secrets_do_not() {
        let mut store = ArtifactStore::default();
        let code = "/cast 盾击\r\n/cast 斩刀\r\n";
        store.capture_report(&serde_json::json!({"artifacts":[{"title":"宏", "language":"jx3_macro", "content":code}]}).to_string());
        assert_eq!(store.items()[0].content, code);
        store.capture_report(&serde_json::json!({"artifacts":[{"title":"secret", "language":"text", "content":"api_key=private-test-fixture-value"}]}).to_string());
        assert_eq!(store.items().len(), 1);
    }
}
