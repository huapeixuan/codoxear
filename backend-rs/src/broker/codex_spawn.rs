use crate::broker::config::BrokerCli;
use crate::broker::runtime::{BrokerEnv, ChildHandle};
use crate::broker::runtime_support::{clean_env, shell_quote};
use std::fs;
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

pub(crate) fn spawn_codex(cli: &BrokerCli, env: &BrokerEnv) -> Result<ChildHandle, String> {
    #[cfg(target_os = "linux")]
    if !stdin_is_terminal() {
        return spawn_piped_process(cli, env);
    }

    let (rows, cols) = crate::broker::pty::terminal_size();
    let argv = codex_exec_argv(cli, env);
    if env.debug {
        eprintln!(
            "codoxear-broker-rs: codex spawn cwd={} argv={:?} codex_bin={} codex_bin_meta={} rows={} cols={}",
            cli.cwd.display(),
            argv,
            env.codex_bin,
            executable_debug(&env.codex_bin),
            rows,
            cols
        );
    }
    let mut master_fd: libc::c_int = -1;
    let mut winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pid = unsafe {
        libc::forkpty(
            &mut master_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize as *mut libc::winsize,
        )
    };
    if pid < 0 {
        return Err(format!(
            "forkpty failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    if pid == 0 {
        child_exec(&argv, &cli.cwd, env, rows, cols);
    }
    unsafe { libc::setpgid(pid, pid) };
    if env.debug {
        eprintln!("codoxear-broker-rs: forkpty child pid={pid} master_fd={master_fd}");
    }
    let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    Ok(ChildHandle::Pty {
        pid: pid as u32,
        master,
    })
}

#[cfg(target_os = "linux")]
fn spawn_piped_process(cli: &BrokerCli, env: &BrokerEnv) -> Result<ChildHandle, String> {
    let argv = codex_exec_argv(cli, env);
    let Some((program, args)) = argv.split_first() else {
        return Err("spawn codex failed: empty argv".to_string());
    };
    if env.debug {
        eprintln!(
            "codoxear-broker-rs: codex non-tty spawn cwd={} argv={:?} codex_bin={} codex_bin_meta={}",
            cli.cwd.display(),
            argv,
            env.codex_bin,
            executable_debug(&env.codex_bin)
        );
    }
    let mut command = Command::new(program);
    command.args(args);
    command.current_dir(&cli.cwd);
    command.env(
        "TERM",
        clean_env("TERM").unwrap_or_else(|| "xterm-256color".to_string()),
    );
    command.env("COLUMNS", "120");
    command.env("LINES", "40");
    command.env("CODEX_HOME", &env.codex_home);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map(|child| ChildHandle::Process { child })
        .map_err(|err| format!("spawn codex failed: {err}"))
}

#[cfg(target_os = "linux")]
fn stdin_is_terminal() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn executable_debug(value: &str) -> String {
    let path = Path::new(value);
    if !path.is_absolute() && !value.contains('/') {
        return "resolved through PATH".to_string();
    }
    match fs::metadata(path) {
        Ok(meta) => format!(
            "exists=true file={} mode={:o}",
            meta.is_file(),
            meta.permissions().mode() & 0o777
        ),
        Err(err) => format!("metadata_error={err}"),
    }
}

fn codex_exec_argv(cli: &BrokerCli, env: &BrokerEnv) -> Vec<String> {
    if env.owner.as_deref() == Some("web") {
        let shell = clean_env("SHELL").unwrap_or_else(|| "/bin/zsh".to_string());
        let mut parts = vec![shell, "-l".to_string(), "-i".to_string(), "-c".to_string()];
        let mut cmd = vec![env.codex_bin.clone()];
        cmd.extend(cli.agent_args.iter().cloned());
        parts.push(format!(
            "exec {}",
            cmd.iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        parts
    } else {
        let mut argv = vec![env.codex_bin.clone()];
        argv.extend(cli.agent_args.iter().cloned());
        argv
    }
}

fn child_exec(argv: &[String], cwd: &Path, env: &BrokerEnv, rows: u16, cols: u16) -> ! {
    unsafe { libc::setpgid(0, 0) };
    if let Err(err) = std::env::set_current_dir(cwd) {
        child_stderr(&format!("codoxear-broker-rs child chdir failed: {err}\n"));
        unsafe { libc::_exit(127) }
    }
    std::env::set_var(
        "TERM",
        clean_env("TERM").unwrap_or_else(|| "xterm-256color".to_string()),
    );
    std::env::set_var("COLUMNS", cols.to_string());
    std::env::set_var("LINES", rows.to_string());
    std::env::set_var("CODEX_HOME", &env.codex_home);
    if argv.is_empty() {
        child_stderr("codoxear-broker-rs child exec failed: empty argv\n");
        unsafe { libc::_exit(127) }
    }
    let cstrings = match argv
        .iter()
        .map(|arg| std::ffi::CString::new(arg.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(_) => {
            child_stderr("codoxear-broker-rs child exec failed: argv contains NUL\n");
            unsafe { libc::_exit(127) }
        }
    };
    let mut ptrs: Vec<*const libc::c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    unsafe {
        libc::execvp(cstrings[0].as_ptr(), ptrs.as_ptr());
        let err = std::io::Error::last_os_error();
        child_stderr(&format!(
            "codoxear-broker-rs child execvp failed for {:?}: {err}\n",
            argv.first().map(String::as_str).unwrap_or("")
        ));
        libc::_exit(127);
    }
}

fn child_stderr(message: &str) {
    unsafe {
        let _ = libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len());
    }
}
