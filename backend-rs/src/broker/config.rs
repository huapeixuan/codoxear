use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerCli {
    pub cwd: PathBuf,
    pub session_file: Option<PathBuf>,
    pub agent_args: Vec<String>,
}

pub fn normalize_backend(raw: Option<&str>) -> String {
    match raw.unwrap_or("codex").trim().to_ascii_lowercase().as_str() {
        "pi" => "pi".to_string(),
        _ => "codex".to_string(),
    }
}

pub fn parse_cli(args: &[String]) -> Result<BrokerCli, String> {
    let mut cwd: Option<PathBuf> = None;
    let mut session_file: Option<PathBuf> = None;
    let mut agent_args: Vec<String> = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--" => {
                agent_args.extend(args[index + 1..].iter().cloned());
                break;
            }
            "--cwd" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--cwd requires a value".to_string());
                };
                cwd = Some(PathBuf::from(value));
            }
            "--session-file" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("--session-file requires a value".to_string());
                };
                session_file = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown broker argument: {other}")),
        }
        index += 1;
    }
    let Some(cwd) = cwd else {
        return Err("--cwd is required".to_string());
    };
    Ok(BrokerCli {
        cwd,
        session_file,
        agent_args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cwd_session_file_and_remainder() {
        let parsed = parse_cli(&[
            "--cwd".into(),
            "/repo".into(),
            "--session-file".into(),
            "/tmp/pi.jsonl".into(),
            "--".into(),
            "-e".into(),
            "bridge.ts".into(),
        ])
        .unwrap();

        assert_eq!(parsed.cwd, PathBuf::from("/repo"));
        assert_eq!(parsed.session_file, Some(PathBuf::from("/tmp/pi.jsonl")));
        assert_eq!(parsed.agent_args, vec!["-e", "bridge.ts"]);
    }
}
