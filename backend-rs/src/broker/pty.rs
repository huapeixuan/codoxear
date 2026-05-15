use crate::broker::runtime::BrokerStateHandle;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;

static SIGWINCH_MASTER_FD: OnceLock<i32> = OnceLock::new();

pub fn start_terminal_bridge(
    mut master: std::fs::File,
    state: BrokerStateHandle,
    stop: Arc<AtomicBool>,
) {
    if !stdin_is_tty() {
        return;
    }
    thread::spawn(move || {
        let fd = libc::STDIN_FILENO;
        let mut old = unsafe { std::mem::zeroed::<libc::termios>() };
        let have_old = unsafe { libc::tcgetattr(fd, &mut old) } == 0;
        if have_old {
            let mut raw = old;
            unsafe { libc::cfmakeraw(&mut raw) };
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };
        }
        let mut buf = [0u8; 4096];
        while !stop.load(Ordering::SeqCst) {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n == 0 {
                if let Ok(mut st) = state.lock() {
                    st.stdin_eof = true;
                }
                stop.store(true, Ordering::SeqCst);
                break;
            }
            if n < 0 {
                break;
            }
            if master.write_all(&buf[..n as usize]).is_err() {
                break;
            }
        }
        if have_old {
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &old) };
        }
    });
}

pub fn install_sigwinch_resize(master_fd: i32) {
    let _ = SIGWINCH_MASTER_FD.set(master_fd);
    resize_pty_to_terminal(master_fd);
    unsafe {
        libc::signal(libc::SIGWINCH, handle_sigwinch as libc::sighandler_t);
    }
}

extern "C" fn handle_sigwinch(_sig: libc::c_int) {
    if let Some(fd) = SIGWINCH_MASTER_FD.get().copied() {
        resize_pty_to_terminal(fd);
    }
}

pub fn resize_pty_to_terminal(master_fd: i32) -> bool {
    let (rows, cols) = terminal_size();
    set_pty_winsize(master_fd, rows, cols)
}

pub fn set_pty_winsize(master_fd: i32, rows: u16, cols: u16) -> bool {
    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe { libc::ioctl(master_fd, libc::TIOCSWINSZ, &winsize) == 0 }
}

pub fn terminal_size() -> (u16, u16) {
    let mut size = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ok = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    if ok && size.ws_row > 0 && size.ws_col > 0 {
        (size.ws_row, size.ws_col)
    } else {
        (40, 120)
    }
}

fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn default_terminal_size_is_non_zero() {
        let (rows, cols) = terminal_size();
        assert!(rows > 0);
        assert!(cols > 0);
    }

    #[test]
    fn set_pty_winsize_rejects_invalid_fd() {
        assert!(!set_pty_winsize(-1, 24, 80));
    }

    #[test]
    fn file_as_raw_fd_stays_available_for_resize_api() {
        let file = tempfile::tempfile().unwrap();
        let _ = set_pty_winsize(file.as_raw_fd(), 24, 80);
    }
}
