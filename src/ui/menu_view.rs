// Start menu content: draws the manifest entries inside the menu window,
// highlighting the keyboard selection.

use std::io::Write;

use super::ansi;
use super::window::Window;
use crate::menu::MenuState;

/// Fit `text` to at most `max` characters, truncating with an ellipsis.
fn fit_width(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

pub fn draw_menu_content(out: &mut impl Write, win: &Window, state: &MenuState) {
    let inner_w = win.width as usize - 2;
    let visible = win.height as usize - 2;
    if inner_w == 0 || visible == 0 {
        return;
    }

    if state.items.is_empty() {
        ansi::move_to(out, win.position_x + 1, win.position_y + 1);
        let hint = fit_width("  (sem itens — edite ~/.manto/menu.json)", inner_w);
        write!(out, "{hint:<inner_w$}").unwrap();
        return;
    }

    for row in 0..visible {
        let idx = state.scroll + row;
        let y = win.position_y + 1 + row as u16;
        ansi::move_to(out, win.position_x + 1, y);

        match state.items.get(idx) {
            Some(item) => {
                let marker = if idx == state.selected { "▶" } else { " " };
                let text = fit_width(&format!("{marker} {}", item.label), inner_w);
                if idx == state.selected {
                    write!(out, "{0}{1:<2$}{3}", ansi::REVERSE, text, inner_w, ansi::RESET).unwrap();
                } else {
                    write!(out, "{text:<inner_w$}").unwrap();
                }
            }
            None => {
                write!(out, "{:1$}", "", inner_w).unwrap();
            }
        }
    }
}