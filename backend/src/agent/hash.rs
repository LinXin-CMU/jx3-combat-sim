use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Convert any serializable value to canonical JSON bytes.
///
/// Object keys are sorted recursively and arrays retain their input order.
/// `serde_json` provides stable JSON number/string encoding for the resulting
/// value, so the output is suitable for cross-process provenance hashes.
pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(value)?;
    serde_json::to_vec(&canonicalize(value))
}

pub fn canonical_sha256<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let bytes = canonical_json_bytes(value)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));

            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, canonicalize(value));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_ignores_object_key_order() {
        let left: Value = serde_json::from_str(r#"{"b":{"d":4,"c":3},"a":1}"#).unwrap();
        let right: Value = serde_json::from_str(r#"{"a":1,"b":{"c":3,"d":4}}"#).unwrap();

        let canonical = canonical_json_bytes(&left).unwrap();
        assert_eq!(canonical, canonical_json_bytes(&right).unwrap());
        assert_eq!(canonical, br#"{"a":1,"b":{"c":3,"d":4}}"#);
        assert_eq!(
            canonical_sha256(&left).unwrap(),
            canonical_sha256(&right).unwrap()
        );
        assert_eq!(
            canonical_sha256(&left).unwrap(),
            "8d463b4d44d84c3a5f01c287245d254181e5d88e0f520c14c325a33422ed9331" // pragma: allowlist secret
        );
    }

    #[test]
    fn canonical_hash_preserves_array_order() {
        let left = serde_json::json!({"sequence": ["盾击", "盾压"]});
        let right = serde_json::json!({"sequence": ["盾压", "盾击"]});

        assert_ne!(
            canonical_sha256(&left).unwrap(),
            canonical_sha256(&right).unwrap()
        );
    }
}
