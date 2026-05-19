use std::collections::{HashSet, VecDeque};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub fn parse_macos_lsof_names(stdout: &str) -> HashSet<PathBuf> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .filter(|path| {
            path.starts_with('/') && path.ends_with(".jsonl") && path.contains("/rollout-")
        })
        .map(PathBuf::from)
        .collect()
}

pub fn parse_child_pids(stdout: &str) -> Vec<u32> {
    stdout
        .split_whitespace()
        .filter_map(|token| token.parse::<u32>().ok())
        .collect()
}

pub fn macos_descendants<F>(root_pid: u32, mut pgrep: F) -> Result<Vec<u32>, String>
where
    F: FnMut(u32) -> Result<String, String>,
{
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([root_pid]);
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        out.push(pid);
        for child in parse_child_pids(&pgrep(pid)?) {
            queue.push_back(child);
        }
    }
    Ok(out)
}

pub fn macos_open_jsonl_logs<F>(pids: &[u32], mut lsof: F) -> Result<HashSet<PathBuf>, String>
where
    F: FnMut(u32) -> Result<String, String>,
{
    let mut out = HashSet::new();
    for pid in pids {
        out.extend(parse_macos_lsof_names(&lsof(*pid)?));
    }
    Ok(out)
}

pub fn proc_descendants(proc_root: &Path, root_pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([root_pid]);
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) {
            continue;
        }
        out.push(pid);
        let children_path = proc_root
            .join(pid.to_string())
            .join("task")
            .join(pid.to_string())
            .join("children");
        let Ok(raw) = fs::read_to_string(children_path) else {
            continue;
        };
        for token in raw.split_whitespace() {
            let parsed: Result<u32, _> = token.parse();
            if let Ok(child) = parsed {
                queue.push_back(child);
            }
        }
    }
    out
}

pub fn proc_open_jsonl_logs(proc_root: &Path, root_pid: u32, current_uid: u32) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    for pid in proc_descendants(proc_root, root_pid) {
        let pid_dir = proc_root.join(pid.to_string());
        if let Ok(meta) = fs::metadata(&pid_dir) {
            if meta.uid() != current_uid {
                continue;
            }
        }
        let fd_dir = pid_dir.join("fd");
        let Ok(entries) = fs::read_dir(fd_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(target) = fs::read_link(entry.path()) else {
                continue;
            };
            let text = target.to_string_lossy();
            if text.starts_with('/') && text.ends_with(".jsonl") && !text.ends_with(" (deleted)") {
                out.insert(target);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn parses_macos_lsof_rollout_names_only() {
        let parsed = parse_macos_lsof_names(
            "p1\nn/tmp/rollout-a.jsonl\nn/tmp/nope.txt\nn/Users/me/.pi/agent/sessions/a.jsonl\n",
        );
        assert!(parsed.contains(Path::new("/tmp/rollout-a.jsonl")));
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn macos_descendants_and_lsof_are_command_runner_testable() {
        let pids = macos_descendants(100, |pid| {
            Ok(match pid {
                100 => "101 102\n".to_string(),
                101 => "103\n".to_string(),
                _ => String::new(),
            })
        })
        .unwrap();
        assert_eq!(pids, vec![100, 101, 102, 103]);

        let logs = macos_open_jsonl_logs(&pids, |pid| {
            Ok(format!(
                "p{pid}\nn/tmp/rollout-{pid}.jsonl\nn/tmp/not-a-log.txt\n"
            ))
        })
        .unwrap();
        assert!(logs.contains(Path::new("/tmp/rollout-100.jsonl")));
        assert!(logs.contains(Path::new("/tmp/rollout-103.jsonl")));
        assert_eq!(logs.len(), 4);
    }

    #[test]
    fn walks_fake_proc_tree_and_fds() {
        let dir = TempDir::new().unwrap();
        let proc_root = dir.path().join("proc");
        let log = dir.path().join("rollout-a.jsonl");
        fs::write(&log, "").unwrap();
        for pid in [100, 101] {
            fs::create_dir_all(
                proc_root
                    .join(pid.to_string())
                    .join("task")
                    .join(pid.to_string()),
            )
            .unwrap();
            fs::create_dir_all(proc_root.join(pid.to_string()).join("fd")).unwrap();
        }
        fs::write(proc_root.join("100/task/100/children"), "101\n").unwrap();
        fs::write(proc_root.join("101/task/101/children"), "\n").unwrap();
        symlink(&log, proc_root.join("101/fd/3")).unwrap();

        let logs = proc_open_jsonl_logs(&proc_root, 100, unsafe { libc::geteuid() });

        assert!(logs.contains(&log));
    }
}
