// terminal.rs — Sequências ANSI/VT100 puras.
//
// Este arquivo não tem dependência de OS. Todas as funções recebem um writer
// genérico e emitem apenas bytes de escape ANSI — funcionam em qualquer
// sistema que implemente a interface Write de os.rs.
//
// Nota: as funções usam `std::io::Write` como bound. Para no_std, defina
// um trait Write equivalente em os.rs e troque o import aqui.

use std::io::Write;

// ── Controle de tela ─────────────────────────────────────────────────────────

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

// ── Cores (SGR) ──────────────────────────────────────────────────────────────

pub const RESET:        &str = "\x1b[0m";
pub const REVERSE:      &str = "\x1b[7m";
pub const FG_BLACK:     &str = "\x1b[30m";
pub const FG_WHITE:     &str = "\x1b[37m";
pub const FG_GREEN:     &str = "\x1b[32m";
pub const BG_WHITE:     &str = "\x1b[47m";
pub const BG_DARK_GREY: &str = "\x1b[100m";

pub fn print_styled(out: &mut impl Write, text: &str, fg: &str, bg: &str) {
    write!(out, "{}{}{}{}", fg, bg, text, RESET).unwrap();
}
