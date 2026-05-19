use crate::broker::config::{normalize_backend, BrokerCli};
use std::process::Command;

pub fn python_exe() -> String {
    std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string())
}

pub fn python_bridge_argv(cli: &BrokerCli, backend_raw: Option<&str>) -> Vec<String> {
    let backend = normalize_backend(backend_raw);
    let module = if backend == "pi" || cli.session_file.is_some() {
        "codoxear.pi_broker"
    } else {
        "codoxear.broker"
    };
    let mut argv = vec![
        python_exe(),
        "-m".to_string(),
        module.to_string(),
        "--cwd".to_string(),
        cli.cwd.to_string_lossy().to_string(),
    ];
    if module == "codoxear.pi_broker" {
        if let Some(session_file) = &cli.session_file {
            argv.extend([
                "--session-file".to_string(),
                session_file.to_string_lossy().to_string(),
            ]);
        }
    }
    argv.push("--".to_string());
    argv.extend(cli.agent_args.iter().cloned());
    argv
}

pub fn exec_python_bridge(cli: BrokerCli) -> ! {
    let argv = python_bridge_argv(
        &cli,
        std::env::var("CODEX_WEB_AGENT_BACKEND").ok().as_deref(),
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = Command::new(&argv[0]).args(&argv[1..]).exec();
        eprintln!("codoxear-broker-rs: exec python bridge failed: {err}");
        std::process::exit(127);
    }
    #[cfg(not(unix))]
    {
        let status = Command::new(&argv[0]).args(&argv[1..]).status();
        match status {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(err) => {
                eprintln!("codoxear-broker-rs: spawn python bridge failed: {err}");
                std::process::exit(127);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::config::BrokerCli;
    use std::path::PathBuf;

    #[test]
    fn codex_bridge_preserves_broker_cli_shape() {
        let cli = BrokerCli {
            cwd: PathBuf::from("/repo"),
            session_file: None,
            agent_args: vec!["--model".into(), "gpt".into()],
        };

        let argv = python_bridge_argv(&cli, Some("codex"));

        assert_eq!(
            argv[1..],
            [
                "-m",
                "codoxear.broker",
                "--cwd",
                "/repo",
                "--",
                "--model",
                "gpt"
            ]
        );
    }

    #[test]
    fn pi_bridge_preserves_session_file_and_agent_args() {
        let cli = BrokerCli {
            cwd: PathBuf::from("/repo"),
            session_file: Some(PathBuf::from("/tmp/pi.jsonl")),
            agent_args: vec!["-e".into(), "bridge.ts".into()],
        };

        let argv = python_bridge_argv(&cli, Some("pi"));

        assert_eq!(
            argv[1..],
            [
                "-m",
                "codoxear.pi_broker",
                "--cwd",
                "/repo",
                "--session-file",
                "/tmp/pi.jsonl",
                "--",
                "-e",
                "bridge.ts"
            ]
        );
    }
}
