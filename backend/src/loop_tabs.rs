//! Named rotation panes. The router's isolated worker owns the storage directory.
use axum::{extract::Query, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Deserialize)]
pub(crate) struct Scope {
    version: String,
    mount: String,
}

fn path(scope: &Scope) -> Result<PathBuf, StatusCode> {
    let valid = |s: &str| !s.is_empty() && s.len() <= 64
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    if !valid(&scope.version) || !valid(&scope.mount) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(crate::user_data_path(&format!("loop_tabs/{}_{}.json", scope.version, scope.mount)))
}

pub(crate) async fn load(Query(scope): Query<Scope>) -> Result<Json<Value>, StatusCode> {
    let path = path(&scope)?;
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Json(Value::Null)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub(crate) async fn save(Query(scope): Query<Scope>, Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let path = path(&scope)?;
    if body.get("schema").and_then(Value::as_u64) != Some(1)
        || !body.get("tabs").is_some_and(Value::is_array) {
        return Err(StatusCode::BAD_REQUEST);
    }
    // Serialize writes within this worker; replacing the file never exposes a partial document.
    static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = WRITE_LOCK.lock().await;
    let temp = path.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let data = serde_json::to_vec(&body)?;
        std::fs::write(&temp, data)?;
        std::fs::rename(&temp, &path)
    };
    write().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({"ok": true})))
}
