use super::Key;
use std::io::Read;
use std::mem::MaybeUninit;
use std::time::{Duration, Instant};

static mut ORIG_TERMIOS: Option<libc::termios> = None;

struct HeldState {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

impl Default for HeldState {
    fn default() -> Self {
        Self { up: false, down: false, left: false, right: false }
    }
}

static mut HELD_ARROWS: HeldState = HeldState::default();
static mut LAST_ARROW_TIME: Option<Instant> = None;

pub fn enable_raw_mode() {
    unsafe {
        let mut t = MaybeUninit::<libc::termios>::uninit();
        libc::tcgetattr(libc::STDIN_FILENO, t.as_mut_ptr());
        let t = t.assume_init();
        ORIG_TERMIOS = Some(t);

        let mut raw = t;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON);
        raw.c_oflag &= !(libc::OPOST);
        raw.c_cc[libc::VMIN as usize]  = 1;
        raw.c_cc[libc::VTIME as usize] = 0;
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
    }
}

pub fn disable_raw_mode() {
    unsafe {
        if let Some(orig) = ORIG_TERMIOS {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &orig);
        }
    }
}

pub fn size() -> (u16, u16) {
    unsafe {
        let mut ws = MaybeUninit::<libc::winsize>::uninit();
        libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, ws.as_mut_ptr());
        let ws = ws.assume_init();
        (ws.ws_col, ws.ws_row)
    }
}

pub fn poll(timeout_ms: u64) -> bool {
    unsafe {
        let mut fds = [libc::pollfd {
            fd:      libc::STDIN_FILENO,
            events:  libc::POLLIN,
            revents: 0,
        }];
        libc::poll(fds.as_mut_ptr(), 1, timeout_ms as libc::c_int) > 0
    }
}

pub fn read_key() -> Key {
    loop {
        let mut buf = [0u8; 1];
        std::io::stdin().read_exact(&mut buf).unwrap();
        match buf[0] {
            3        => return Key::CtrlC,
            4        => return Key::CtrlD,
            5        => return Key::CtrlE,
            6        => return Key::CtrlF,
            10       => return Key::CtrlJ,
            11       => return Key::CtrlK,
            12       => return Key::CtrlL,
            14       => return Key::CtrlN,
            16       => return Key::CtrlP,
            17       => return Key::CtrlQ,
            23       => return Key::CtrlW,
            22       => return Key::CtrlV,
            24       => return Key::CtrlX,
            26       => return Key::CtrlZ,
            20       => return Key::CtrlT,
            8 | 127  => return Key::Backspace,
            9        => return Key::Tab,
            13       => return Key::Enter,
            27 => {
                if poll(10) {
                    let mut first = [0u8; 1];
                    std::io::stdin().read_exact(&mut first).unwrap();
                    if first[0] == b'[' {
                        let mut second = [0u8; 1];
                        std::io::stdin().read_exact(&mut second).unwrap();
                        match second[0] {
                            b'A' => {
                                unsafe { HELD_ARROWS.up = true; LAST_ARROW_TIME = Some(Instant::now()); }
                                return Key::Up;
                            }
                            b'B' => {
                                unsafe { HELD_ARROWS.down = true; LAST_ARROW_TIME = Some(Instant::now()); }
                                return Key::Down;
                            }
                            b'C' => {
                                unsafe { HELD_ARROWS.right = true; LAST_ARROW_TIME = Some(Instant::now()); }
                                return Key::Right;
                            }
                            b'D' => {
                                unsafe { HELD_ARROWS.left = true; LAST_ARROW_TIME = Some(Instant::now()); }
                                return Key::Left;
                            }
                            b'F' => return Key::End,
                            b'H' => return Key::Home,
                            b'0'..=b'9' => {
                                let mut params = vec![second[0]];
                                loop {
                                    let mut next = [0u8; 1];
                                    std::io::stdin().read_exact(&mut next).unwrap();
                                    match next[0] {
                                        b'~' => {
                                            match params.as_slice() {
                                                b"3" => return Key::Delete,
                                                b"3;5" => return Key::CtrlDelete,
                                                b"5" => return Key::PageUp,
                                                b"6" => return Key::PageDown,
                                                _ => continue,
                                            }
                                        }
                                        b'A'..=b'Z' => {
                                            match (params.as_slice(), next[0]) {
                                                (b"1;3", b'A') | (b"3", b'A') => return Key::AltUp,
                                                (b"1;3", b'B') | (b"3", b'B') => return Key::AltDown,
                                                (b"1;3", b'C') | (b"3", b'C') => return Key::AltRight,
                                                (b"1;3", b'D') | (b"3", b'D') => return Key::AltLeft,
                                                (b"1;2", b'A') => return Key::ShiftUp,
                                                (b"1;2", b'B') => return Key::ShiftDown,
                                                (b"1;2", b'C') => return Key::ShiftRight,
                                                (b"1;2", b'D') => return Key::ShiftLeft,
                                                _ => continue,
                                            }
                                        }
                                        b'a'..=b'z' => continue,
                                        _ => params.push(next[0]),
                                    }
                                }
                            }
                            _    => continue,
                        }
                    } else {
                        match first[0] {
                            b'h' | b'H' => return Key::AltH,
                            b'r' | b'R' => return Key::AltR,
                            b'v' | b'V' => return Key::AltV,
                            _ => continue,
                        }
                    }
                } else {
                    return Key::Escape;
                }
            }
            b if b.is_ascii_graphic() || b == b' ' => return Key::Char(b as char),
            b if b >= 0x80 => {
                if let Some(c) = read_utf8_char(b) {
                    if !c.is_control() {
                        return Key::Char(c);
                    }
                }
                continue;
            }
            _ => continue,
        }
    }
}

/// Decode a UTF-8 character from the first byte already read, consuming the
/// required continuation bytes from stdin.
fn read_utf8_char(first: u8) -> Option<char> {
    let (extra, mut cp) = match first {
        0xC2..=0xDF => (1, (first & 0x1F) as u32),
        0xE0..=0xEF => (2, (first & 0x0F) as u32),
        0xF0..=0xF4 => (3, (first & 0x07) as u32),
        _ => return None,
    };
    for _ in 0..extra {
        let mut byte = [0u8; 1];
        std::io::stdin().read_exact(&mut byte).ok()?;
        if byte[0] & 0xC0 != 0x80 {
            return None;
        }
        cp = (cp << 6) | ((byte[0] & 0x3F) as u32);
    }
    char::from_u32(cp)
}

pub fn held_arrow_keys() -> super::HeldArrowKeys {
    unsafe {
        let now = Instant::now();
        if let Some(last) = LAST_ARROW_TIME {
            if now.duration_since(last) > Duration::from_millis(500) {
                HELD_ARROWS = HeldState::default();
                LAST_ARROW_TIME = None;
            }
        }
        super::HeldArrowKeys {
            up: HELD_ARROWS.up,
            down: HELD_ARROWS.down,
            left: HELD_ARROWS.left,
            right: HELD_ARROWS.right,
        }
    }
}

// ── Clipboard (external tools, best-effort) ──────────────────────────────────
// Uses xclip/xsel (X11) or wl-copy/wl-paste (Wayland) when present; Manto keeps
// an in-memory fallback if none is available.

fn tool_ok(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn clipboard_set(text: &str) -> bool {
    const SETTERS: [(&str, &[&str]); 3] = [
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("wl-copy", &[]),
    ];
    for (tool, args) in SETTERS {
        if !tool_ok(tool) {
            continue;
        }
        let mut cmd = std::process::Command::new(tool);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if let Ok(mut child) = cmd.spawn() {
            use std::io::Write;
            let ok = child
                .stdin
                .take()
                .map(|mut si| si.write_all(text.as_bytes()).and_then(|_| si.flush()).is_ok())
                .unwrap_or(false);
            let _ = child.wait();
            if ok {
                return true;
            }
        }
    }
    false
}

pub fn clipboard_get() -> Option<String> {
    const GETTERS: [(&str, &[&str]); 3] = [
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
        ("wl-paste", &[]),
    ];
    for (tool, args) in GETTERS {
        if !tool_ok(tool) {
            continue;
        }
        let out = std::process::Command::new(tool)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(o) = out {
            if o.status.success() && !o.stdout.is_empty() {
                let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                while s.ends_with('\n') || s.ends_with('\r') {
                    s.pop();
                }
                return Some(s);
            }
        }
    }
    None
}
