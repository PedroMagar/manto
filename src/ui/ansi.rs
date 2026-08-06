// Pure ANSI/VT100 escape sequences.
#![allow(dead_code)]
//
// OS-independent: every function takes a generic writer and emits ANSI
// escape bytes only, so it works on any system implementing the Write
// interface from the os layer.

use std::io::Write;

// ── Screen control ───────────────────────────────────────────────────────────

pub fn clear(out: &mut impl Write) {
    write!(out, "\x1b[2J\x1b[H").unwrap();
}

pub fn move_to(out: &mut impl Write, x: u16, y: u16) {
    write!(out, "\x1b[{};{}H", y + 1, x + 1).unwrap();
}

pub fn hide_cursor(out: &mut impl Write) {
    write!(out, "\x1b[?25l").unwrap();
}

pub fn show_cursor(out: &mut impl Write) {
    write!(out, "\x1b[?25h").unwrap();
}

pub fn enter_alt_screen(out: &mut impl Write) {
    write!(out, "\x1b[?1049h").unwrap();
}

pub fn leave_alt_screen(out: &mut impl Write) {
    write!(out, "\x1b[?1049l").unwrap();
}

// ── Colors (SGR) ─────────────────────────────────────────────────────────────

pub const RESET:        &str = "\x1b[0m";
pub const REVERSE:      &str = "\x1b[7m";
