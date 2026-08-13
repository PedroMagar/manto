// Pure ANSI/VT100 escape sequences.
//
// OS-independent: every function takes a generic writer and emits ANSI
// escape bytes only, so it works on any system implementing the Write
// interface from the os layer.
#![allow(dead_code)]

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

pub const RESET: &str = "\x1b[0m";
pub const REVERSE: &str = "\x1b[7m";
pub const REVERSE_OFF: &str = "\x1b[27m";
pub const BOLD: &str = "\x1b[1m";
pub const BOLD_OFF: &str = "\x1b[22m";
pub const DIM: &str = "\x1b[2m";
pub const ITALIC: &str = "\x1b[3m";
pub const ITALIC_OFF: &str = "\x1b[23m";
pub const UNDERLINE: &str = "\x1b[4m";
pub const UNDERLINE_OFF: &str = "\x1b[24m";
pub const BLINK: &str = "\x1b[5m";
pub const BLINK_OFF: &str = "\x1b[25m";
pub const HIDDEN: &str = "\x1b[8m";
pub const HIDDEN_OFF: &str = "\x1b[28m";
pub const STRIKE: &str = "\x1b[9m";
pub const STRIKE_OFF: &str = "\x1b[29m";
pub const DIM_OFF: &str = "\x1b[22m";

/// Emit the SGR for `next` relative to `prev` (None for a fresh row). Only the
/// changes are written; keeps the grid renderer's per-cell overhead low.
pub fn sgr(
    out: &mut impl Write,
    prev: Option<&crate::terminal_emulator::Style>,
    next: &crate::terminal_emulator::Style,
) {
    use crate::terminal_emulator::{Color, Style};

    fn is_default(s: &Style) -> bool {
        s.attrs.is_empty() && s.fg == Color::Default && s.bg == Color::Default
    }

    let Some(p) = prev else {
        // No prior style: absolute emit.
        let mut s = String::from(RESET);
        append_style(&mut s, &Style::default(), next);
        write!(out, "{s}").unwrap();
        return;
    };

    if is_default(next) {
        if is_default(p) {
            return;
        }
        write!(out, "{RESET}").unwrap();
        return;
    }

    let mut s = String::new();
    if is_default(p) {
        s.push_str(RESET);
    }
    append_style(&mut s, p, next);
    write!(out, "{s}").unwrap();
}

fn append_style(
    s: &mut String,
    prev: &crate::terminal_emulator::Style,
    next: &crate::terminal_emulator::Style,
) {
    use crate::terminal_emulator::Attributes;
    for (flag, off, on) in [
        (Attributes::BOLD, BOLD_OFF, BOLD),
        (Attributes::DIM, DIM_OFF, DIM),
        (Attributes::ITALIC, ITALIC_OFF, ITALIC),
        (Attributes::UNDERLINE, UNDERLINE_OFF, UNDERLINE),
        (Attributes::BLINK, BLINK_OFF, BLINK),
        (Attributes::HIDDEN, HIDDEN_OFF, HIDDEN),
        (Attributes::STRIKE, STRIKE_OFF, STRIKE),
    ] {
        if prev.attrs.has(flag) != next.attrs.has(flag) {
            s.push_str(if next.attrs.has(flag) { on } else { off });
        }
    }
    if prev.attrs.has(Attributes::REVERSE) != next.attrs.has(Attributes::REVERSE) {
        s.push_str(if next.attrs.has(Attributes::REVERSE) {
            REVERSE
        } else {
            REVERSE_OFF
        });
    }
    if prev.fg != next.fg {
        s.push_str(&fg_sgr(next.fg));
    }
    if prev.bg != next.bg {
        s.push_str(&bg_sgr(next.bg));
    }
}

fn fg_sgr(color: crate::terminal_emulator::Color) -> String {
    use crate::terminal_emulator::Color;
    match color {
        Color::Default => "\x1b[39m".to_string(),
        Color::Indexed(n @ 0..=7) => format!("\x1b[{}m", 30 + n),
        Color::Indexed(n @ 8..=15) => format!("\x1b[{}m", 90 + (n - 8)),
        Color::Indexed(n) => format!("\x1b[38;5;{n}m"),
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
    }
}

fn bg_sgr(color: crate::terminal_emulator::Color) -> String {
    use crate::terminal_emulator::Color;
    match color {
        Color::Default => "\x1b[49m".to_string(),
        Color::Indexed(n @ 0..=7) => format!("\x1b[{}m", 40 + n),
        Color::Indexed(n @ 8..=15) => format!("\x1b[{}m", 100 + (n - 8)),
        Color::Indexed(n) => format!("\x1b[48;5;{n}m"),
        Color::Rgb(r, g, b) => format!("\x1b[48;2;{r};{g};{b}m"),
    }
}
