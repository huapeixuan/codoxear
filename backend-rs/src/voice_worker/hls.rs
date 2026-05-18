use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use std::fs;
use std::path::{Path as FsPath, PathBuf};

use crate::app_state::AppState;

pub const HLS_MAX_SEGMENTS: usize = 18;
pub const HLS_TARGET_DURATION_SECONDS: u32 = 12;
pub const HLS_KEEPALIVE_SECONDS: f64 = 6.0;
pub const HLS_SILENCE_SECONDS: f64 = 6.0;

pub trait HlsMediaRunner: Send + Sync {
    fn append_aac_as_segments(&self, input: &FsPath, output_pattern: &FsPath)
        -> Result<(), String>;
    fn segment_duration(&self, segment: &FsPath) -> Result<f64, String>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct HlsSegment {
    pub seq: usize,
    pub name: String,
    pub duration: f64,
    pub path: PathBuf,
}

pub fn empty_playlist(media_sequence: usize) -> String {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{HLS_TARGET_DURATION_SECONDS}\n#EXT-X-MEDIA-SEQUENCE:{media_sequence}\n"
    )
}

pub fn render_playlist(media_sequence: usize, segments: &[HlsSegment]) -> String {
    if segments.is_empty() {
        return empty_playlist(media_sequence);
    }
    let target = segments
        .iter()
        .map(|segment| segment.duration.ceil() as u32)
        .max()
        .unwrap_or(HLS_TARGET_DURATION_SECONDS)
        .max(HLS_TARGET_DURATION_SECONDS);
    let first_seq = segments
        .first()
        .map(|segment| segment.seq)
        .unwrap_or(media_sequence);
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:{first_seq}\n"
    );
    for segment in segments {
        out.push_str(&format!(
            "#EXTINF:{:.3},\nsegments/{}\n",
            segment.duration, segment.name
        ));
    }
    out
}

pub fn safe_segment_path(app_dir: &FsPath, segment: &str) -> Option<PathBuf> {
    let name = FsPath::new(segment)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if name != segment || !name.ends_with(".ts") {
        return None;
    }
    Some(app_dir.join("audio").join("segments").join(name))
}

pub async fn audio_playlist(State(state): State<AppState>) -> Response {
    let path = state.config.app_dir.join("audio").join("live.m3u8");
    match fs::read(&path) {
        Ok(body) => bytes_response(body, "application/vnd.apple.mpegurl"),
        Err(_) => not_found(),
    }
}

pub async fn audio_segment(State(state): State<AppState>, Path(segment): Path<String>) -> Response {
    let Some(path) = safe_segment_path(&state.config.app_dir, &segment) else {
        return not_found();
    };
    let root = state.config.app_dir.join("audio").join("segments");
    let Ok(root_canon) = root.canonicalize() else {
        return not_found();
    };
    let Ok(path_canon) = path.canonicalize() else {
        return not_found();
    };
    if !path_canon.starts_with(&root_canon) || !path_canon.is_file() {
        return not_found();
    }
    match fs::read(path_canon) {
        Ok(body) => bytes_response(body, "video/mp2t"),
        Err(_) => not_found(),
    }
}

fn not_found() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

fn bytes_response(raw: Vec<u8>, content_type: &'static str) -> Response {
    let len = raw.len();
    let mut response = Response::new(Body::from(raw));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).unwrap(),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, HeaderValue::from_static("0"));
    response
}
