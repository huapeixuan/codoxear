use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PullRequest {
    pub number: i64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub is_draft: bool,
    pub head_ref_name: String,
}

impl PullRequest {
    pub fn to_summary(&self) -> Value {
        let state = if self.is_draft && self.state == "OPEN" {
            "DRAFT"
        } else {
            self.state.as_str()
        };
        json!({"number": self.number, "state": state})
    }

    pub fn to_dict(&self) -> Value {
        json!({
            "number": self.number,
            "title": self.title,
            "state": self.state,
            "url": self.url,
            "is_draft": self.is_draft,
            "head_ref_name": self.head_ref_name,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RepoContext {
    pub git_branch: Option<String>,
    pub pr: Option<PullRequest>,
    pub availability: String,
    pub cwd: String,
}

impl RepoContext {
    pub fn to_detail_dict(&self) -> Value {
        json!({
            "cwd": self.cwd,
            "git_branch": self.git_branch,
            "pr": self.pr.as_ref().map(PullRequest::to_dict),
            "availability": self.availability,
        })
    }

    pub fn pr_summary(&self) -> Option<Value> {
        self.pr.as_ref().map(PullRequest::to_summary)
    }
}

#[derive(Clone)]
struct CacheEntry {
    context: RepoContext,
    expires_at: Instant,
}

#[derive(Clone)]
struct GhAuthState {
    ok: bool,
    checked_at: Instant,
}

type GhAuthCache = Mutex<Option<GhAuthState>>;

static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
static GH_AUTH: OnceLock<GhAuthCache> = OnceLock::new();

pub fn resolve_repo_context(cwd: &Path, refresh: bool) -> RepoContext {
    let original_cwd = cwd.to_string_lossy().to_string();
    let Ok(resolved) = cwd.canonicalize() else {
        return not_repo_context(original_cwd);
    };
    if !resolved.is_dir() {
        return not_repo_context(original_cwd);
    }
    let key = resolved.to_string_lossy().to_string();
    if !refresh {
        if let Some(entry) = cache().lock().unwrap().get(&key).cloned() {
            if entry.expires_at > Instant::now() {
                return entry.context;
            }
        }
    }

    let context = compute_context(&resolved);
    let ttl = if matches!(context.availability.as_str(), "ok" | "no-pr") {
        pr_ttl()
    } else {
        branch_ttl()
    };
    cache().lock().unwrap().insert(
        key,
        CacheEntry {
            context: context.clone(),
            expires_at: Instant::now() + ttl,
        },
    );
    context
}

pub fn current_git_branch(cwd: &Path) -> Option<String> {
    resolve_repo_context(cwd, false).git_branch
}

pub fn reset_caches_for_tests() {
    cache().lock().unwrap().clear();
    *gh_auth().lock().unwrap() = None;
}

fn compute_context(cwd: &Path) -> RepoContext {
    let (branch, is_repo) = resolve_branch(cwd);
    if !is_repo {
        return RepoContext {
            git_branch: None,
            pr: None,
            availability: "not-a-repo".to_string(),
            cwd: cwd.to_string_lossy().to_string(),
        };
    }
    let (pr, availability) = resolve_pr(cwd);
    RepoContext {
        git_branch: branch,
        pr,
        availability,
        cwd: cwd.to_string_lossy().to_string(),
    }
}

fn resolve_branch(cwd: &Path) -> (Option<String>, bool) {
    let (code, out, _) = run(
        &["git", "rev-parse", "--is-inside-work-tree"],
        cwd,
        branch_timeout(),
    );
    if code != 0 || !out.trim().eq_ignore_ascii_case("true") {
        return (None, false);
    }
    let (code, out, _) = run(
        &["git", "symbolic-ref", "--quiet", "--short", "HEAD"],
        cwd,
        branch_timeout(),
    );
    if code == 0 {
        let branch = out.trim();
        if !branch.is_empty() {
            return (Some(branch.to_string()), true);
        }
    }
    let (code, out, _) = run(
        &["git", "rev-parse", "--short", "HEAD"],
        cwd,
        branch_timeout(),
    );
    if code == 0 {
        let sha = out.trim();
        return ((!sha.is_empty()).then(|| sha.to_string()), true);
    }
    (None, true)
}

fn resolve_pr(cwd: &Path) -> (Option<PullRequest>, String) {
    if which("gh").is_none() || !check_gh_auth(false) {
        return (None, "no-gh".to_string());
    }
    let (code, out, err) = run(
        &[
            "gh",
            "pr",
            "view",
            "--json",
            "number,title,state,url,isDraft,headRefName",
        ],
        cwd,
        pr_timeout(),
    );
    if code == -1 {
        return (None, "error".to_string());
    }
    if code != 0 {
        let message = format!("{}{}", err, out).to_ascii_lowercase();
        if message.contains("no pull requests found")
            || message.contains("no pr")
            || message.contains("could not find")
        {
            return (None, "no-pr".to_string());
        }
        return (None, "error".to_string());
    }
    let Ok(data) = serde_json::from_str::<Value>(&out) else {
        return (None, "error".to_string());
    };
    let number = data
        .get("number")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if number <= 0 {
        return (None, "no-pr".to_string());
    }
    let pr = PullRequest {
        number,
        title: value_string(data.get("title")),
        state: value_string(data.get("state")).to_ascii_uppercase(),
        url: value_string(data.get("url")),
        is_draft: data
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        head_ref_name: value_string(data.get("headRefName")),
    };
    (Some(pr), "ok".to_string())
}

fn check_gh_auth(refresh: bool) -> bool {
    if which("gh").is_none() {
        *gh_auth().lock().unwrap() = Some(GhAuthState {
            ok: false,
            checked_at: Instant::now(),
        });
        return false;
    }
    if !refresh {
        if let Some(state) = gh_auth().lock().unwrap().clone() {
            if state.checked_at.elapsed() < gh_auth_ttl() {
                return state.ok;
            }
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (code, _, _) = run(&["gh", "auth", "status"], &cwd, pr_timeout());
    let ok = code == 0;
    *gh_auth().lock().unwrap() = Some(GhAuthState {
        ok,
        checked_at: Instant::now(),
    });
    ok
}

pub fn run_git_capture(
    cwd: &Path,
    args: &[&str],
    timeout: Duration,
    byte_cap: usize,
) -> (i32, String, String) {
    let mut command = Command::new("git");
    command.args(args).current_dir(cwd);
    run_command(command, timeout, byte_cap)
}

fn run(args: &[&str], cwd: &Path, timeout: Duration) -> (i32, String, String) {
    let mut command = Command::new(args[0]);
    command.args(&args[1..]).current_dir(cwd);
    run_command(command, timeout, 256 * 1024)
}

fn run_command(mut command: Command, _timeout: Duration, byte_cap: usize) -> (i32, String, String) {
    // std::process has no built-in timeout. Phase 2 keeps subprocess byte caps and
    // short command scopes; a later async process wrapper can enforce wall timeout.
    match command.output() {
        Ok(output) => (
            output.status.code().unwrap_or(1),
            capped_utf8(&output.stdout, byte_cap),
            capped_utf8(&output.stderr, byte_cap),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            (-1, String::new(), format!("missing executable: {err}"))
        }
        Err(err) => (-1, String::new(), err.to_string()),
    }
}

fn capped_utf8(raw: &[u8], cap: usize) -> String {
    let bytes = if raw.len() > cap { &raw[..cap] } else { raw };
    String::from_utf8_lossy(bytes).to_string()
}

fn not_repo_context(cwd: String) -> RepoContext {
    RepoContext {
        git_branch: None,
        pr: None,
        availability: "not-a-repo".to_string(),
        cwd,
    }
}

fn value_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn env_duration(name: &str, default: f64) -> Duration {
    let seconds = std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(default);
    Duration::from_secs_f64(seconds)
}

fn branch_timeout() -> Duration {
    env_duration("CODEX_WEB_BRANCH_TIMEOUT_S", 2.0)
}

fn pr_timeout() -> Duration {
    env_duration("CODEX_WEB_PR_TIMEOUT_S", 4.0)
}

fn branch_ttl() -> Duration {
    env_duration("CODEX_WEB_BRANCH_TTL_S", 30.0)
}

fn pr_ttl() -> Duration {
    env_duration("CODEX_WEB_PR_TTL_S", 120.0)
}

fn gh_auth_ttl() -> Duration {
    env_duration("CODEX_WEB_GH_AUTH_TTL_S", 300.0)
}

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn gh_auth() -> &'static GhAuthCache {
    GH_AUTH.get_or_init(|| Mutex::new(None))
}
