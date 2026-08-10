use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::io::RawFd;
use std::sync::mpsc::Sender;
use std::thread;

use super::TerminalUpdate;

// ── PTY / process helpers via libc FFI ────────────────────────────────────────

const O_RDWR:   i32 = 0o0002;
const O_NOCTTY: i32 = 0o0110; // 0o0100 (O_NOCTTY) | 0o0010 (O_CLOEXEC)
const TIOCGWINSZ: usize = 0x5413;
const TIOCSWINSZ: usize = 0x5414;

unsafe extern "C" {
    fn posix_openpt(oflag: i32) -> RawFd;
    fn grantpt(fd: RawFd) -> i32;
    fn unlockpt(fd: RawFd) -> i32;
    fn ptsname(fd: RawFd) -> *const i8;
    fn fork() -> i32;
    fn execve(path: *const i8, argv: *const *const i8, envp: *const *const i8) -> i32;
    fn setsid() -> i32;
    fn chdir(path: *const i8) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    fn dup2(oldfd: RawFd, newfd: RawFd) -> RawFd;
    fn close(fd: RawFd) -> i32;
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn open(path: *const i8, flags: i32) -> RawFd;
}

#[cfg(not(target_os = "linux"))]
unsafe fn open(path: *const i8, flags: i32) -> RawFd {
    // Fallback non-Linux implementation using posix_openpt on the slave path is
    // not portable; Linux is the primary Unix target. On other Unixes authors
    // should adapt this. We still attempt open(2) via libc where declared.
    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd", target_os = "dragonfly"))]
    {
        extern "C" {
            fn open(path: *const libc::c_char, flags: libc::c_int, ...) -> libc::c_int;
        }
        open(path, flags)
    }
    #[cfg(not(any(target_os = "macos", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd", target_os = "dragonfly")))]
    {
        let _ = (path, flags);
        -1 as RawFd
    }
}

/// Set the PTY window size via TIOCSWINSZ.
fn pty_set_size(master_fd: RawFd, rows: u16, cols: u16) {
    #[repr(C)]
    struct Winsize {
        ws_row:    u16,
        ws_col:    u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }
    let ws = Winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe {
        libc::ioctl(master_fd, TIOCSWINSZ as _, &ws);
    }
}

// ── Platform state ────────────────────────────────────────────────────────────

pub struct PlatformCommand {
    master_fd:   RawFd,
    pid:         i32,
    input_tx:    Sender<Vec<u8>>,
    input_thread: Option<thread::JoinHandle<()>>,
    closed:      bool,
}

impl PlatformCommand {
    pub fn try_wait(&mut self) -> Option<i32> {
        let mut status: libc::c_int = 0;
        unsafe {
            let ret = libc::waitpid(self.pid, &mut status, libc::WNOHANG);
            if ret == self.pid {
                self.closed = true;
                if libc::WIFEXITED(status) {
                    Some(libc::WEXITSTATUS(status) as i32)
                } else if libc::WIFSIGNALED(status) {
                    Some(128 + libc::WTERMSIG(status) as i32)
                } else {
                    Some(0)
                }
            } else {
                None
            }
        }
    }

    pub fn kill(&mut self) -> bool {
        unsafe {
            // Kill the process group so children (e.g. a running command) die too.
            let _ = kill(-self.pid, libc::SIGKILL);
            let _ = kill(self.pid, libc::SIGKILL);
        }
        true
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.input_tx
            .send(data.to_vec())
            .map_err(|_| "input channel disconnected".to_string())?;
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        pty_set_size(self.master_fd, rows, cols);
        Ok(())
    }

    pub fn is_real_pty(&self) -> bool {
        true
    }
}

impl Drop for PlatformCommand {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.input_tx.send(Vec::new()); // stop the input thread
        if let Some(t) = self.input_thread.take() {
            let _ = t.join();
        }
        unsafe {
            if !self.closed {
                let _ = libc::waitpid(self.pid, std::ptr::null_mut(), 0); // reap
            }
            let _ = close(self.master_fd);
        }
    }
}

// ── Spawn a persistent shell session ──────────────────────────────────────────

/// Spawn `command` (a shell path) attached to a fresh PTY.
///
/// The child process runs the shell interactively:
///   execvp(shell, shell -i)
///
/// `command` is treated as the shell program to exec.
pub fn spawn(command: &str, cwd: &str, tx: Sender<TerminalUpdate>) -> Result<PlatformCommand, String> {
    let argv = [command.to_string(), "-i".to_string()];
    spawn_pty(&argv, cwd, tx)
}

/// Spawn a program session (interactive app). The command line runs through
/// `/bin/sh -lc`, so arguments parse like a typed command and bare names
/// resolve through the shell.
pub fn spawn_app(program: &str, cwd: &str, tx: Sender<TerminalUpdate>) -> Result<PlatformCommand, String> {
    if program.trim().is_empty() {
        let shell = crate::app::terminal::default_shell();
        return spawn(&shell, cwd, tx);
    }
    let argv = ["/bin/sh".to_string(), "-lc".to_string(), program.to_string()];
    spawn_pty(&argv, cwd, tx)
}

/// Spawn a program with explicit arguments attached to a fresh PTY
/// (`execvp(program, program args...)`, no shell in between). This is the
/// `TerminalBackend::spawn` platform path.
pub fn spawn_argv(program: &str, args: &[String], cwd: &str, tx: Sender<TerminalUpdate>) -> Result<PlatformCommand, String> {
    let mut argv = Vec::with_capacity(args.len() + 1);
    argv.push(program.to_string());
    argv.extend_from_slice(args);
    spawn_pty(&argv, cwd, tx)
}

fn spawn_pty(argv: &[String], cwd: &str, tx: Sender<TerminalUpdate>) -> Result<PlatformCommand, String> {
    let master_fd = unsafe { posix_openpt(O_RDWR | O_NOCTTY) };
    if master_fd < 0 {
        return Err("posix_openpt failed".to_string());
    }

    if unsafe { grantpt(master_fd) } != 0 {
        unsafe { close(master_fd) };
        return Err("grantpt failed".to_string());
    }
    if unsafe { unlockpt(master_fd) } != 0 {
        unsafe { close(master_fd) };
        return Err("unlockpt failed".to_string());
    }

    let slave_path_ptr = unsafe { ptsname(master_fd) };
    if slave_path_ptr.is_null() {
        unsafe { close(master_fd) };
        return Err("ptsname failed".to_string());
    }
    let slave_path = unsafe { CStr::from_ptr(slave_path_ptr) }.to_string_lossy().into_owned();

    // Initial size from the host terminal.
    let (rows, cols) = unsafe {
        let mut ws = std::mem::zeroed::<libc::winsize>();
        libc::ioctl(libc::STDOUT_FILENO, TIOCGWINSZ as _, &mut ws);
        (ws.ws_row, ws.ws_col)
    };
    pty_set_size(master_fd, rows.max(2), cols.max(2));

    let exec_c: Vec<CString> = argv.iter()
        .map(|arg| CString::new(arg.as_bytes()))
        .collect::<Result<_, _>>()
        .map_err(|_| "argument has interior NUL".to_string())?;
    let slave_c = CString::new(slave_path.as_bytes()).map_err(|_| "slave path has interior NUL".to_string())?;
    let cwd_c = CString::new(cwd.as_bytes()).map_err(|_| "cwd has interior NUL".to_string())?;

    let pid = unsafe {
        match fork() {
            -1 => {
                close(master_fd);
                return Err("fork failed".to_string());
            }
            0 => {
                // ── Child ──
                let _ = setsid();
                let slave_fd = open(slave_c.as_ptr(), O_RDWR | O_NOCTTY);
                if slave_fd < 0 {
                    libc::_exit(127);
                }
                let _ = dup2(slave_fd, libc::STDIN_FILENO);
                let _ = dup2(slave_fd, libc::STDOUT_FILENO);
                let _ = dup2(slave_fd, libc::STDERR_FILENO);
                if slave_fd > libc::STDERR_FILENO {
                    let _ = close(slave_fd);
                }
                let _ = chdir(cwd_c.as_ptr());
                let mut ptrs: Vec<*const i8> = exec_c.iter().map(|c| c.as_ptr()).collect();
                ptrs.push(std::ptr::null());
                execve(exec_c[0].as_ptr(), ptrs.as_ptr(), std::ptr::null());
                libc::_exit(127);
            }
            pid => pid,
        }
    };

    // ── Parent ──
    // The master end is made non-blocking: the reader thread below parks in
    // poll() and reads only what the fd reports ready, so a stuck child can
    // never block the drain. The input thread already retries EAGAIN.
    let flags = unsafe { libc::fcntl(master_fd, libc::F_GETFL) };
    if flags >= 0 {
        unsafe {
            let _ = libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }

    // Input forwarding thread: read from the channel, write to the PTY master.
    let (input_tx, input_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let input_thread = thread::spawn(move || {
        while let Ok(data) = input_rx.recv() {
            if data.is_empty() { break; }
            let mut off: usize = 0;
            while off < data.len() {
                unsafe {
                    let ret = libc::write(master_fd, data[off..].as_ptr() as *const libc::c_void, data.len() - off);
                    if ret > 0 {
                        off += ret as usize;
                    } else if ret < 0 {
                        let err = io::Error::last_os_error();
                        if err.kind() == io::ErrorKind::WouldBlock {
                            libc::usleep(500);
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    });

    // Output reader thread: parks in poll() and reads only what the master
    // reports as ready, then forwards the raw bytes as they arrive. EOF (the
    // child closed its end) or a hard read error ends the loop. Polling a
    // non-blocking fd guarantees the read below never blocks, so one busy
    // session cannot stall the others.
    let tx_out = tx.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            let mut pfd = libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, -1) };
            if ready < 0 {
                // Poll failed: the fd is gone, nothing more to read.
                break;
            }
            if ready == 0 || pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) == 0 {
                continue;
            }
            unsafe {
                let ret = libc::read(master_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len());
                if ret > 0 {
                    let n = ret as usize;
                    if tx_out.send(TerminalUpdate::Output(buf[..n].to_vec())).is_err() {
                        break;
                    }
                } else if ret == 0 {
                    // EOF: the child closed its end of the pty.
                    break;
                } else {
                    // EAGAIN after a poll wakeup is a benign race on a
                    // non-blocking fd; loop back to poll().
                    let err = io::Error::last_os_error();
                    if err.kind() != io::ErrorKind::WouldBlock {
                        break;
                    }
                }
            }
        }
        let _ = tx_out.send(TerminalUpdate::Closed);
    });

    Ok(PlatformCommand {
        master_fd,
        pid,
        input_tx,
        input_thread: Some(input_thread),
        closed: false,
    })
}
