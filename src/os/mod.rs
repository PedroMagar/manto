// Host OS abstraction layer.
//
// Everything that depends on the concrete host OS lives here:
//   - Writer  : output (stdout now; framebuffer/serial on a custom OS)
//   - Clock   : time (Instant now; a hardware register on a custom OS)
//   - Key     : keyboard events
//   - platform: raw mode, terminal size, polling and key decoding
//
// Porting to a new OS means replacing the platform module while keeping the
// same public interface; the rest of the codebase stays unchanged.

use std::io::{self, Write};
use std::time::{Duration, Instant};

// ── Writer ────────────────────────────────────────────────────────────────────
// On a custom OS: write to a text framebuffer, serial port, etc.

pub struct Writer(io::BufWriter<io::Stdout>);

impl Writer {
    pub fn new() -> Self {
        Self(io::BufWriter::new(io::stdout()))
    }
}

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

// ── Clock ─────────────────────────────────────────────────────────────────────
// On a custom OS: read a hardware register (TSC, RTC, MMIO timer, etc.)

pub struct Clock(Instant);

impl Clock {
    pub fn now() -> Self {
        Self(Instant::now())
    }

    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

// ── Key ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    ShiftUp,
    ShiftDown,
    ShiftLeft,
    ShiftRight,
    Tab,
    Enter,
    Backspace,
    Delete,
    Escape,
    End,
    Home,
    F1,
    PageUp,
    PageDown,
    Ctrl1,
    Ctrl2,
    Ctrl3,
    Ctrl4,
    AltUp,
    AltDown,
    AltLeft,
    AltRight,
    CtrlDelete,
    CtrlC,
    CtrlD,
    CtrlE,
    CtrlF,
    CtrlH,
    CtrlJ,
    CtrlK,
    CtrlL,
    CtrlN,
    CtrlP,
    CtrlQ,
    CtrlW,
    CtrlV,
    CtrlX,
    CtrlZ,
    AltH,
    AltR,
    AltV,
    CtrlEnter,
    CtrlT,
    CtrlM,
    Mouse(MouseEvent),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldArrowKeys {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

// ── Mouse ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// Button pressed (or wheel scrolled).
    Press,
    /// Button released (ends a click/drag).
    Release,
    /// Pointer moved without any button held.
    Move,
    /// Pointer moved while a button is held (click-drag).
    Drag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    /// 1-based column, matching the terminal reports.
    pub x: u16,
    /// 1-based row.
    pub y: u16,
    pub kind: MouseAction,
    /// Set for wheel events; for Move/Release events the value is Left.
    pub button: MouseButton,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

// ── Clipboard bridge (best-effort, no deps) ──────────────────────────────────
// Windows uses Win32 FFI; Unix shells out to xclip/xsel/wl-copy when present.

pub use platform::{clipboard_set, clipboard_get};

/// Search PATH for an executable by name, honoring PATHEXT. App-execution
/// alias directories (WindowsApps) are skipped: those reparse points cannot
/// be started by `CreateProcessW` and direct the search to a real binary.
#[cfg(windows)]
pub fn find_on_path(program: &str) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());

    for dir in std::env::split_paths(&path) {
        if dir.to_string_lossy().contains("WindowsApps") {
            continue;
        }
        let exact = dir.join(program);
        if exact.is_file() {
            return Some(exact.to_string_lossy().to_string());
        }
        for ext in pathext.split(';') {
            if ext.is_empty() {
                continue;
            }
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    None
}

// ── Platform ──────────────────────────────────────────────────────────────────
//
// Every platform module exports:
//   enable_raw_mode()      - put the terminal in raw mode
//   disable_raw_mode()     - restore the original mode
//   size() -> (u16, u16)   - (width, height) in cells
//   poll(ms: u64) -> bool  - true if input is available within the timeout
//   read_key() -> Key      - read and decode the next key
//   held_arrow_keys()      - arrow keys currently held (for quadrant snapping)
//   clipboard_set/get      - best-effort OS clipboard

pub use platform::{enable_raw_mode, disable_raw_mode, size, poll, read_key, held_arrow_keys};

#[cfg(windows)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_on_path_resolves_system_executables() {
        let found = find_on_path("cmd.exe").expect("cmd.exe is reachable on PATH");
        assert!(found.ends_with("cmd.exe"), "wrong candidate: {found}");
        assert!(!found.to_ascii_lowercase().contains("windowsapps"));
    }

    #[test]
    fn find_on_path_respects_pathext() {
        let found = find_on_path("powershell").expect("powershell.exe reachable on PATH");
        assert!(found.to_ascii_lowercase().ends_with("powershell.exe"));
    }
}

#[cfg(unix)]
#[path = "unix.rs"]
mod platform;

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
