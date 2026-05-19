use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use std::fs;
use std::path::{Path as FsPath, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app_state::AppState;

pub const HLS_MAX_SEGMENTS: usize = 18;
pub const HLS_TARGET_DURATION_SECONDS: u32 = 12;
pub const HLS_KEEPALIVE_SECONDS: f64 = 6.0;
pub const HLS_SILENCE_SECONDS: f64 = 6.0;

pub trait HlsMediaRunner: Send + Sync {
    fn append_aac_as_segments(&self, input: &FsPath, output_pattern: &FsPath)
        -> Result<(), String>;
    fn append_silence_segment(&self, output: &FsPath) -> Result<(), String>;
    fn segment_duration(&self, segment: &FsPath) -> Result<f64, String>;
}

#[derive(Debug, Clone, Default)]
pub struct FfmpegHlsMediaRunner;

impl HlsMediaRunner for FfmpegHlsMediaRunner {
    fn append_aac_as_segments(
        &self,
        input: &FsPath,
        output_pattern: &FsPath,
    ) -> Result<(), String> {
        let output = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(input)
            .args([
                "-vn",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-f",
                "segment",
                "-segment_time",
                "6",
                "-segment_format",
                "mpegts",
                "-reset_timestamps",
                "1",
            ])
            .arg(output_pattern)
            .output()
            .map_err(|err| format!("ffmpeg failed: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "ffmpeg failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }

    fn append_silence_segment(&self, output_path: &FsPath) -> Result<(), String> {
        let output = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=24000:cl=mono",
                "-t",
                &HLS_SILENCE_SECONDS.to_string(),
                "-c:a",
                "aac",
                "-b:a",
                "32k",
                "-f",
                "mpegts",
            ])
            .arg(output_path)
            .output()
            .map_err(|err| format!("ffmpeg failed: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "ffmpeg failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }

    fn segment_duration(&self, segment: &FsPath) -> Result<f64, String> {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(segment)
            .output()
            .map_err(|err| format!("ffprobe failed: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "ffprobe failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        parse_duration(&String::from_utf8_lossy(&output.stdout))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HlsSegment {
    pub seq: usize,
    pub name: String,
    pub duration: f64,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct MergedHlsStream {
    root_dir: PathBuf,
    segments: Vec<HlsSegment>,
    next_seq: usize,
    last_append_ts: f64,
    last_error: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HlsSnapshot {
    pub segment_count: usize,
    pub media_sequence: usize,
    pub last_error: String,
}

impl MergedHlsStream {
    pub fn new(root_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(root_dir.join("segments"))
            .map_err(|err| format!("create HLS segments dir: {err}"))?;
        let stream = Self {
            root_dir,
            segments: Vec::new(),
            next_seq: 1,
            last_append_ts: 0.0,
            last_error: String::new(),
        };
        stream.rewrite_playlist()?;
        Ok(stream)
    }

    pub fn snapshot(&self) -> HlsSnapshot {
        HlsSnapshot {
            segment_count: self.segments.len(),
            media_sequence: self
                .segments
                .first()
                .map(|segment| segment.seq)
                .unwrap_or(self.next_seq),
            last_error: self.last_error.clone(),
        }
    }

    pub fn append_audio<R: HlsMediaRunner>(
        &mut self,
        runner: &R,
        message_id: &str,
        audio_bytes: &[u8],
    ) -> Result<f64, String> {
        fs::create_dir_all(self.segments_dir())
            .map_err(|err| format!("create HLS segments dir: {err}"))?;
        let prefix = segment_prefix(message_id);
        let input = self.segments_dir().join(format!("{prefix}.aac"));
        fs::write(&input, audio_bytes)
            .map_err(|err| format!("write {}: {err}", input.display()))?;
        let pattern = self.segments_dir().join(format!("{prefix}-part-%03d.ts"));
        let result = self.append_audio_inner(runner, &prefix, &input, &pattern);
        let _ = fs::remove_file(&input);
        for chunk in glob_part_segments(&self.segments_dir(), &prefix) {
            let _ = fs::remove_file(chunk);
        }
        result
    }

    pub fn append_silence<R: HlsMediaRunner>(
        &mut self,
        runner: &R,
        force: bool,
        now_ts: f64,
    ) -> Result<bool, String> {
        if !force
            && self.last_append_ts > 0.0
            && (now_ts - self.last_append_ts) < HLS_KEEPALIVE_SECONDS
        {
            return Ok(false);
        }
        fs::create_dir_all(self.segments_dir())
            .map_err(|err| format!("create HLS segments dir: {err}"))?;
        let (seq, name, path) = self.reserve_segment("silence");
        runner
            .append_silence_segment(&path)
            .inspect_err(|err| self.last_error = err.clone())?;
        let duration = runner
            .segment_duration(&path)
            .inspect_err(|err| self.last_error = err.clone())?;
        self.store_segment(seq, name, path, duration.max(0.2), now_ts)?;
        Ok(true)
    }

    pub fn reset(&mut self) -> Result<(), String> {
        let old_paths = self
            .segments
            .iter()
            .map(|segment| segment.path.clone())
            .collect::<Vec<_>>();
        self.segments.clear();
        self.last_error.clear();
        self.last_append_ts = 0.0;
        self.rewrite_playlist()?;
        for path in old_paths {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    fn append_audio_inner<R: HlsMediaRunner>(
        &mut self,
        runner: &R,
        prefix: &str,
        input: &FsPath,
        pattern: &FsPath,
    ) -> Result<f64, String> {
        runner
            .append_aac_as_segments(input, pattern)
            .inspect_err(|err| self.last_error = err.clone())?;
        let chunks = glob_part_segments(&self.segments_dir(), prefix);
        if chunks.is_empty() {
            let err = "ffmpeg produced no HLS segments".to_string();
            self.last_error = err.clone();
            return Err(err);
        }
        let mut total = 0.0;
        for chunk in chunks {
            let duration = match runner.segment_duration(&chunk) {
                Ok(duration) => duration.max(0.2),
                Err(error) if error.contains("invalid ffprobe duration: N/A") => {
                    let _ = fs::remove_file(&chunk);
                    continue;
                }
                Err(error) => {
                    self.last_error = error.clone();
                    return Err(error);
                }
            };
            let (seq, name, path) = self.reserve_segment(prefix);
            fs::rename(&chunk, &path).map_err(|err| {
                format!("rename {} to {}: {err}", chunk.display(), path.display())
            })?;
            total += duration;
            self.store_segment(seq, name, path, duration, unix_now())?;
        }
        if total <= 0.0 {
            let err = "ffmpeg produced no valid HLS segments".to_string();
            self.last_error = err.clone();
            return Err(err);
        }
        self.last_error.clear();
        Ok(total)
    }

    fn reserve_segment(&mut self, prefix: &str) -> (usize, String, PathBuf) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let name = format!("{seq:06}-{}.ts", segment_prefix(prefix));
        let path = self.segments_dir().join(&name);
        (seq, name, path)
    }

    fn store_segment(
        &mut self,
        seq: usize,
        name: String,
        path: PathBuf,
        duration: f64,
        now_ts: f64,
    ) -> Result<(), String> {
        self.segments.push(HlsSegment {
            seq,
            name,
            duration,
            path,
        });
        self.segments.sort_by_key(|segment| segment.seq);
        while self.segments.len() > HLS_MAX_SEGMENTS {
            let old = self.segments.remove(0);
            let _ = fs::remove_file(old.path);
        }
        self.last_append_ts = now_ts;
        self.rewrite_playlist()
    }

    fn rewrite_playlist(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root_dir).map_err(|err| format!("create HLS root: {err}"))?;
        fs::write(
            self.root_dir.join("live.m3u8"),
            render_playlist(self.next_seq, &self.segments),
        )
        .map_err(|err| format!("write live.m3u8: {err}"))
    }

    fn segments_dir(&self) -> PathBuf {
        self.root_dir.join("segments")
    }
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

pub fn parse_duration(raw: &str) -> Result<f64, String> {
    let value = raw.trim();
    value
        .parse::<f64>()
        .map(|duration| duration.max(0.2))
        .map_err(|_| format!("invalid ffprobe duration: {value}"))
}

fn segment_prefix(raw: &str) -> String {
    let cleaned = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(12)
        .collect::<String>();
    if cleaned.is_empty() {
        "audio".to_string()
    } else {
        cleaned
    }
}

fn glob_part_segments(dir: &FsPath, prefix: &str) -> Vec<PathBuf> {
    let pattern_prefix = format!("{}-part-", segment_prefix(prefix));
    let mut out = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| {
                            name.starts_with(&pattern_prefix) && name.ends_with(".ts")
                        })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
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
