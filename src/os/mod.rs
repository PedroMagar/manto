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

#[derive(Debug)]
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
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HeldArrowKeys {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

// ── Clipboard bridge (best-effort, no deps) ──────────────────────────────────
// Windows uses Win32 FFI; Unix shells out to xclip/xsel/wl-copy when present.

pub use platform::{clipboard_set, clipboard_get};

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

#[cfg(unix)]
#[path = "unix.rs"]
mod platform;

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
