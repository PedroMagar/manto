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

const HELD_ARROWS_NONE: HeldState = HeldState { up: false, down: false, left: false, right: false };

static mut HELD_ARROWS: HeldState = HELD_ARROWS_NONE;
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

    // Mouse reporting: button events (1000), press+drag (1002), any-motion
    // hover (1003) and SGR encoding (1006) so coordinates are exact.
    write_mouse_mode(true);
}

pub fn disable_raw_mode() {
    unsafe {
        if let Some(orig) = ORIG_TERMIOS {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &orig);
        }
    }
    write_mouse_mode(false);
}

/// Enable (`true`) or disable (`false`) DEC mouse tracking on stdout.
fn write_mouse_mode(enable: bool) {
    use std::io::Write;
    let out = std::io::stdout();
    let mut out = out.lock();
    for code in [1000u16, 1002, 1003, 1006] {
        let (on, off) = (format!("\x1b[?{code}h"), format!("\x1b[?{code}l"));
        let _ = out.write_all(if enable { on.as_bytes() } else { off.as_bytes() });
    }
    let _ = out.flush();
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
                            b'<' => {
                                // SGR mouse: ESC[<b;x;yM (press) / ...m (release)
                                let mut params = Vec::new();
                                loop {
                                    let mut byte = [0u8; 1];
                                    std::io::stdin().read_exact(&mut byte).unwrap();
                                    match byte[0] {
                                        b'M' | b'm' => {
                                            let text = String::from_utf8_lossy(&params).into_owned();
                                            return decode_sgr_mouse(&text, byte[0]);
                                        }
                                        b'0'..=b'9' | b';' => params.push(byte[0]),
                                        _ => break,
                                    }
                                }
                                continue;
                            }
                            b'M' => {
                                // X10 mouse: ESC[M Cb Cx Cy
                                let mut bytes = [0u8; 3];
                                std::io::stdin().read_exact(&mut bytes).unwrap();
                                return decode_x10_mouse(bytes);
                            }
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

// ── Mouse decoding (X10 + SGR) ────────────────────────────────────────────────
//
// The button code carries the button, action and modifiers in its bits:
//   - bits 0-1 : 0 = left, 1 = middle, 2 = right
//   - bit  2   : shift
//   - bit  3   : meta/alt
//   - bit  4   : ctrl
//   - bit  5   : action (0 = press, 1 = release) — X10 only
//   - bit  6   : motion (drag / hover move)
//   - values 64/65 (0x40/0x41 with no button): wheel up / wheel down

/// Decode an SGR mouse report: "b;x;y" plus the final byte (`M` press, `m`
/// release). Coordinates are 1-based.
fn decode_sgr_mouse(params: &str, final_byte: u8) -> super::Key {
    let mut it = params.split(';');
    let code: u32 = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let x: u16 = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let y: u16 = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);

    let pressed = final_byte == b'M';
    let motion = code & 0x40 != 0;
    let (button, kind) = mouse_from_code(code, pressed, motion);

    super::Key::Mouse(super::MouseEvent {
        x,
        y,
        kind,
        button,
        shift: code & 0x4 != 0,
        alt:   code & 0x8 != 0,
        ctrl:  code & 0x10 != 0,
    })
}

/// Decode an X10 mouse report: the three bytes after ESC[M are already
/// offset by 32 (+1 for column/row). Coordinates are 1-based.
fn decode_x10_mouse(bytes: [u8; 3]) -> super::Key {
    let code = bytes[0].saturating_sub(32);
    let x = bytes[1].saturating_sub(32);
    let y = bytes[2].saturating_sub(32);

    // X10 encodes release directly in the button byte (bit 5).
    let pressed = code & 0x20 == 0;
    let motion = code & 0x40 != 0;
    let (button, kind) = mouse_from_code(code as u32, pressed, motion);

    super::Key::Mouse(super::MouseEvent {
        x: u16::from(x),
        y: u16::from(y),
        kind,
        button,
        shift: code & 0x4 != 0,
        alt:   code & 0x8 != 0,
        ctrl:  code & 0x10 != 0,
    })
}

fn mouse_from_code(code: u32, pressed: bool, motion: bool) -> (super::MouseButton, super::MouseAction) {
    use super::{MouseAction, MouseButton};

    // Wheel: 0x40/0x41 (64/65) with no button bits set.
    if code & 0x43 == 0x40 {
        let (button, kind) = if code == 0x41 {
            (MouseButton::WheelDown, MouseAction::Press)
        } else {
            (MouseButton::WheelUp, MouseAction::Press)
        };
        let action = if pressed { kind } else { MouseAction::Release };
        return (button, action);
    }

    let button = match code & 0x3 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        _ => MouseButton::Right,
    };

    if motion {
        let kind = if pressed {
            // With a wheel/motion bit set and no button, it's a hover move.
            if code & 0x3 == 3 { MouseAction::Move } else { MouseAction::Drag }
        } else {
            MouseAction::Move
        };
        (button, kind)
    } else {
        (button, if pressed { MouseAction::Press } else { MouseAction::Release })
    }
}

#[cfg(test)]
mod mouse_tests {
    use super::*;

    #[test]
    fn sgr_left_click_is_press() {
        let Key::Mouse(ev) = decode_sgr_mouse("0;12;7", b'M') else { panic!() };
        assert_eq!(ev.x, 12);
        assert_eq!(ev.y, 7);
        assert_eq!(ev.button, super::MouseButton::Left);
        assert_eq!(ev.kind, super::MouseAction::Press);
        assert!(!ev.shift && !ev.ctrl && !ev.alt);
    }

    #[test]
    fn sgr_release_maps_to_release() {
        let Key::Mouse(ev) = decode_sgr_mouse("0;12;7", b'm') else { panic!() };
        assert_eq!(ev.kind, super::MouseAction::Release);
        assert_eq!(ev.button, super::MouseButton::Left);
    }

    #[test]
    fn sgr_modifiers_and_drag() {
        // 29 = 0b11101: left button + shift (0x4) + alt (0x8) + ctrl (0x10).
        let Key::Mouse(ev) = decode_sgr_mouse("29;5;9", b'M') else { panic!() };
        assert_eq!(ev.button, super::MouseButton::Left);
        assert_eq!(ev.kind, super::MouseAction::Press);
        assert!(ev.shift && ev.alt && ev.ctrl);

        // 66 = 0x42: motion bit (0x40) + right button -> drag.
        let Key::Mouse(ev) = decode_sgr_mouse("66;5;9", b'M') else { panic!() };
        assert_eq!(ev.button, super::MouseButton::Right);
        assert_eq!(ev.kind, super::MouseAction::Drag);
    }

    #[test]
    fn sgr_motion_events() {
        let Key::Mouse(ev) = decode_sgr_mouse("35;3;3", b'M') else { panic!() };
        // 35 = 0b100011 -> motion + right button? 35 & 3 = 3 -> treated as Move.
        assert_eq!(ev.kind, super::MouseAction::Move);
    }

    #[test]
    fn sgr_wheel_up_down() {
        let Key::Mouse(ev) = decode_sgr_mouse("64;1;1", b'M') else { panic!() };
        assert_eq!(ev.button, super::MouseButton::WheelUp);
        let Key::Mouse(ev) = decode_sgr_mouse("65;1;1", b'M') else { panic!() };
        assert_eq!(ev.button, super::MouseButton::WheelDown);
    }

    #[test]
    fn x10_left_click_and_release() {
        // Press: code 0 + 32, col 10 + 32, row 5 + 32.
        let Key::Mouse(ev) = decode_x10_mouse([32, 42, 37]) else { panic!() };
        assert_eq!(ev.button, super::MouseButton::Left);
        assert_eq!(ev.kind, super::MouseAction::Press);
        assert_eq!(ev.x, 10);
        assert_eq!(ev.y, 5);

        // Release: code |= 0x20 (32) -> 32 | 32 = 64.
        let Key::Mouse(ev) = decode_x10_mouse([64, 42, 37]) else { panic!() };
        assert_eq!(ev.kind, super::MouseAction::Release);
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
