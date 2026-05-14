use axum::http::StatusCode;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

static STATE_FILE_LOCKS: OnceLock<Mutex<HashMap<String, &'static Mutex<()>>>> = OnceLock::new();

pub fn with_state_file_lock<T>(
    path: &Path,
    f: impl FnOnce() -> Result<T, (StatusCode, String)>,
) -> Result<T, (StatusCode, String)> {
    let lock = {
        let key = path.to_string_lossy().into_owned();
        let registry = STATE_FILE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut registry = registry.lock().map_err(|_| lock_poisoned(path))?;
        *registry
            .entry(key)
            .or_insert_with(|| Box::leak(Box::new(Mutex::new(()))))
    };
    let _guard = lock.lock().map_err(|_| lock_poisoned(path))?;
    f()
}

pub fn read_value(path: &Path) -> Result<Option<Value>, (StatusCode, String)> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).map(Some).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("parse {}: {err}", path.display()),
            )
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read {}: {err}", path.display()),
        )),
    }
}

pub fn read_object_file(path: &Path) -> Result<Map<String, Value>, (StatusCode, String)> {
    match read_value(path)? {
        Some(Value::Object(object)) => Ok(object),
        Some(_) | None => Ok(Map::new()),
    }
}

pub fn read_string_file(
    path: &Path,
    clean: impl Fn(&str) -> String,
) -> Result<HashMap<String, String>, (StatusCode, String)> {
    Ok(read_object_file(path)?
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|raw| (key, clean(raw))))
        .filter(|(_, value)| !value.is_empty())
        .collect())
}

pub fn read_array_file<T>(
    path: &Path,
    clean: impl Fn(Vec<Value>) -> Vec<T>,
) -> Result<HashMap<String, Vec<T>>, (StatusCode, String)> {
    Ok(read_object_file(path)?
        .into_iter()
        .filter_map(|(key, value)| value.as_array().cloned().map(|items| (key, clean(items))))
        .collect())
}

pub fn write_object_file(
    path: &Path,
    object: &Map<String, Value>,
) -> Result<(), (StatusCode, String)> {
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let mut sorted = Map::new();
    for key in keys {
        if let Some(value) = object.get(&key) {
            sorted.insert(key, value.clone());
        }
    }
    write_value(path, &Value::Object(sorted))
}

pub fn write_string_file(
    path: &Path,
    values: &HashMap<String, String>,
) -> Result<(), (StatusCode, String)> {
    let mut keys = values.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let mut object = Map::new();
    for key in keys {
        if let Some(value) = values.get(&key) {
            if !value.is_empty() {
                object.insert(key, Value::String(value.clone()));
            }
        }
    }
    write_value(path, &Value::Object(object))
}

pub fn write_array_file(
    path: &Path,
    values: &HashMap<String, Vec<Value>>,
) -> Result<(), (StatusCode, String)> {
    let mut keys = values.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let mut object = Map::new();
    for key in keys {
        if let Some(items) = values.get(&key) {
            if !items.is_empty() {
                object.insert(key, Value::Array(items.clone()));
            }
        }
    }
    write_value(path, &Value::Object(object))
}

pub fn write_value(path: &Path, value: &Value) -> Result<(), (StatusCode, String)> {
    let parent = path.parent().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("missing parent for {}", path.display()),
    ))?;
    fs::create_dir_all(parent).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create {}: {err}", parent.display()),
        )
    })?;
    let tmp = path.with_extension("json.tmp");
    let raw = serde_json::to_string_pretty(value)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        + "\n";
    fs::write(&tmp, raw).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write {}: {err}", tmp.display()),
        )
    })?;
    fs::rename(&tmp, path).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("rename {}: {err}", path.display()),
        )
    })?;
    if let Ok(parent_file) = fs::File::open(parent) {
        let _ = parent_file.sync_all();
    }
    Ok(())
}

fn lock_poisoned(path: &Path) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("state file lock poisoned for {}", path.display()),
    )
}
