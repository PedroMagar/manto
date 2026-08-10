use std::io::Write;

use super::ansi;
use super::window::Window;
use super::{TERMINAL_INPUT_PREFIX, draw_scrollbar};
use crate::cmd::{CommandEntry, CommandStatus};

/// Slice `line` to `width` characters starting at `scroll_x`.
pub(super) fn slice_line(line: &str, scroll_x: usize, width: usize) -> String {
    line.chars().skip(scroll_x).take(width).collect()
}

fn command_output_line(entry: &CommandEntry, index: usize, line: &str) -> String {
    let last_idx = entry.output_lines.len().saturating_sub(1);
    let branch = if index == last_idx { "└─ " } else { "├─ " };
    let suffix = if index == last_idx && !matches!(entry.status, CommandStatus::Complete) {
        " (running)"
    } else {
        ""
    };
    format!("  │ {}{}{}", branch, line, suffix)
}

pub fn terminal_content_width(path: &str, commands: &[CommandEntry]) -> usize {
    let mut max_width = path.chars().count() + 3;
    let mut last_cwd: Option<&str> = None;

    for entry in commands {
        if !entry.cwd.is_empty() && last_cwd != Some(entry.cwd.as_str()) {
            max_width = max_width.max(entry.cwd.chars().count());
        }
        max_width = max_width.max(format!("  ├─┬ {}", entry.command).chars().count());
        for (index, line) in entry.output_lines.iter().enumerate() {
            max_width = max_width.max(command_output_line(entry, index, line).chars().count());
        }
        if !entry.cwd.is_empty() {
            last_cwd = Some(entry.cwd.as_str());
        }
    }

    max_width
}

/// Draw the content of a Terminal window over the already rendered chrome.
///
/// Inner layout (top to bottom):
///   rows 1 .. h-4  : command history (same priority rules as the global panel)
///   row  h-3       : ├─ path ─────────────────────────────────────────────────┤
///   row  h-2       : │ .> input                                               │
///   row  h-1       : └────────────────────────────────────────────────────────┘  (chrome)
///
/// Requires `win.height >= 5`; otherwise it is a no-op.
pub fn draw_terminal_content(
    out:          &mut impl Write,
    win:          &Window,
    path:         &str,
    commands:     &[CommandEntry],
    panel_scroll: usize,
) {
    if win.height < 5 { return; }

    let lx       = win.position_x;
    let ty       = win.position_y;
    let inner_w  = (win.width - 2) as usize;
    let content_w = terminal_content_width(path, commands).max(inner_w);
    let has_hscroll = content_w > inner_w;
    let content_h = win.height.saturating_sub(if has_hscroll { 5 } else { 4 }) as usize;
    let max_scroll = content_w.saturating_sub(inner_w);
    let scroll_x = (win.scroll_x as usize).min(max_scroll);

    // Command history
    let rows = if commands.is_empty() {
        vec![]
    } else {
        let blocks  = super::panel::build_blocks(commands, content_w);
        let sr_len  = super::panel::total_rows(&blocks);
        let scroll  = panel_scroll.min(sr_len.saturating_sub(content_h));

        if scroll == 0 {
            super::panel::build_priority_rows(&blocks, content_h).0
        } else {
            let clipped = super::panel::clip_newest(&blocks, scroll);
            let flat    = super::panel::flatten(&clipped);
            let start   = flat.len().saturating_sub(content_h);
            let mut rows: Vec<String> = flat[start..].iter().map(|r| r.text.clone()).collect();
            if let Some(first) = flat.get(start) {
                if !rows.is_empty() { rows[0] = first.header.clone(); }
            }
            rows
        }
    };

    for (i, row) in rows.iter().enumerate() {
        ansi::move_to(out, lx + 1, ty + 1 + i as u16);
        let display = slice_line(row, scroll_x, inner_w);
        write!(out, "{:<width$}", display, width = inner_w).unwrap();
    }
    for i in rows.len()..content_h {
        ansi::move_to(out, lx + 1, ty + 1 + i as u16);
        write!(out, "{:<width$}", "", width = inner_w).unwrap();
    }

    // Path separator
    let path_y = ty + win.height - if has_hscroll { 4 } else { 3 };
    ansi::move_to(out, lx, path_y);
    if path.is_empty() {
        write!(out, "├{:─<1$}┤", "", inner_w).unwrap();
    } else {
        let label = format!("── {} ", path);
        let display = slice_line(&label, scroll_x, inner_w);
        let fill  = inner_w.saturating_sub(display.chars().count());
        write!(out, "├{}{:─<fill$}┤", display, "", fill = fill).unwrap();
    }

    // Input row (prefix only; the actual content is drawn by the main loop)
    let input_y   = ty + win.height - if has_hscroll { 3 } else { 2 };
    let prefix_len = TERMINAL_INPUT_PREFIX.chars().count();
    ansi::move_to(out, lx + 1, input_y);
    write!(out, "{}{:<width$}", TERMINAL_INPUT_PREFIX, "", width = inner_w.saturating_sub(prefix_len)).unwrap();
}

/// Draw the raw output of a persistent shell session.
///
/// Inner layout (top to bottom):
///   rows 1 .. h-4  : shell output (scrollable; right column = vertical scrollbar)
///   row  h-3       : ├─ path ────────────────────────────────────────────────┤
///   row  h-2       : │ .> (input row; content of the focused terminal)       │
///   row  h-1       : (bottom border, rendered by the chrome)
///
/// When `repl` is Some (a fullscreen REPL is active, e.g. Python ">>>"), the
/// window becomes a "full terminal": no path separator and the REPL prompt
/// replaces the " .> " bar on the input row.
pub fn draw_shell_content(
    out:          &mut impl Write,
    win:          &Window,
    lines:        &[String],
    panel_scroll: usize,
    repl:         Option<&str>,
) {
    if win.height < 5 { return; }

    let lx      = win.position_x;
    let ty      = win.position_y;
    let inner_w = (win.width - 2) as usize;

    // In REPL mode the path row is gained as output area.
    let content_h = win.height.saturating_sub(if repl.is_some() { 3 } else { 4 }) as usize;
    if content_h == 0 { return; }

    let has_vscroll = lines.len() > content_h;
    let content_w = inner_w.saturating_sub(if has_vscroll { 1 } else { 0 });
    let max_scroll = lines.len().saturating_sub(content_h);
    let scroll     = panel_scroll.min(max_scroll);

    if has_vscroll {
        // Overflow: sliding window over the lines, following the end by default.
        let end   = lines.len().saturating_sub(scroll);
        let start = end.saturating_sub(content_h);
        let mut row = 0usize;
        for i in start..end {
            ansi::move_to(out, lx + 1, ty + 1 + row as u16);
            let display = slice_line(&lines[i], 0, content_w);
            write!(out, "{:<width$}", display, width = content_w).unwrap();
            row += 1;
        }
        for r in row..content_h {
            ansi::move_to(out, lx + 1, ty + 1 + r as u16);
            write!(out, "{:<width$}", "", width = content_w).unwrap();
        }

        // `draw_scrollbar` expects `scroll` as "position from the top"
        // (0 = top, max = bottom); here `scroll` is "how far up from the end"
        // (0 = newest at the bottom), so it is inverted.
        draw_scrollbar(
            out,
            lx + inner_w as u16,
            ty + 1,
            ty + content_h as u16,
            lines.len(),
            content_h,
            max_scroll.saturating_sub(scroll),
        );
    } else {
        // No overflow: align the lines to the bottom (next to the input row),
        // leaving the space above empty. There is nowhere to scroll.
        let offset = content_h - lines.len();
        for r in 0..offset {
            ansi::move_to(out, lx + 1, ty + 1 + r as u16);
            write!(out, "{:<width$}", "", width = content_w).unwrap();
        }
        for (i, line) in lines.iter().enumerate() {
            ansi::move_to(out, lx + 1, ty + 1 + (offset + i) as u16);
            let display = slice_line(line, 0, content_w);
            write!(out, "{:<width$}", display, width = content_w).unwrap();
        }
    }

    // Not in REPL mode: path separator
    if repl.is_none() {
        let path_y = ty + win.height - 3;
        ansi::move_to(out, lx, path_y);
        write!(out, "├{:─<1$}┤", "", inner_w).unwrap();
    }

    // Input row (prefix; the focused terminal content is drawn over it by the
    // main loop)
    let input_y = ty + win.height - 2;
    let prefix = repl.unwrap_or(TERMINAL_INPUT_PREFIX);
    let prefix_len = prefix.chars().count();
    ansi::move_to(out, lx + 1, input_y);
    write!(out, "{}{:<width$}", prefix, "", width = inner_w.saturating_sub(prefix_len)).unwrap();
}

/// Render an interactive terminal's emulator grid into the window interior.
/// `panel_scroll > 0` scrolls the viewport up into the scrollback; the cursor
/// cell is emphasized when `show_cursor`.
pub fn draw_emulator_content(
    out: &mut impl Write,
    win: &Window,
    term: &crate::terminal_emulator::Terminal,
    panel_scroll: usize,
    show_cursor: bool,
) {
    use crate::terminal_emulator::{Attributes, Style};

    if win.height < 5 { return; }

    let lx = win.position_x;
    let ty = win.position_y;
    let inner_w = (win.width - 2) as usize;
    let inner_h = (win.height - 2) as usize;
    let cols = (term.cols() as usize).min(inner_w);
    let rows = (term.rows() as usize).min(inner_h);
    if cols == 0 || rows == 0 { return; }

    let total = term.total_lines();
    let visible = rows;
    let max_scroll = total.saturating_sub(visible);
    let scroll = panel_scroll.min(max_scroll);
    let view_top_abs = total - visible - scroll;

    for row in 0..visible {
        let abs = view_top_abs + row;
        let line = term.line_at(abs);
        ansi::move_to(out, lx + 1, ty + 1 + row as u16);

        let mut prev_style: Option<Style> = None;
        for col in 0..cols {
            let cell = line[col];
            ansi::sgr(out, prev_style.as_ref(), &cell.style);
            prev_style = Some(cell.style);
            write!(out, "{}", cell.ch).unwrap();
        }
        // Pad the rest of the window width with default style.
        ansi::sgr(out, prev_style.as_ref(), &Style::default());
        for _ in cols..inner_w {
            write!(out, " ").unwrap();
        }
    }

    // Clear window interior rows below the emulator height (if any).
    for i in rows..inner_h {
        ansi::move_to(out, lx + 1, ty + 1 + i as u16);
        write!(out, "{:<width$}", "", width = inner_w).unwrap();
    }

    // Emulator cursor, when visible in the current viewport.
    if show_cursor && term.cursor_visible() {
        let (cx, cy) = term.cursor_pos();
        let cursor_abs = term.scrollback_len() + cy as usize;
        if (cx as usize) < cols && cursor_abs >= view_top_abs && cursor_abs < view_top_abs + visible {
            let vrow = cursor_abs - view_top_abs;
            let cell = term.line_at(cursor_abs)[cx as usize];
            let mut cursor_style = cell.style;
            cursor_style.attrs.set(Attributes::REVERSE, true);
            cursor_style.attrs.set(Attributes::BOLD, true);
            ansi::move_to(out, lx + 1 + cx as u16, ty + 1 + vrow as u16);
            ansi::sgr(out, None, &cursor_style);
            write!(out, "{}", cell.ch).unwrap();
            ansi::sgr(out, Some(&cursor_style), &Style::default());
        }
    }

    if max_scroll > 0 {
        let sb_x = lx + inner_w as u16;
        // draw_scrollbar expects scroll "from the top"; panel_scroll is
        // "how far up from the end", so it is inverted here.
        draw_scrollbar(out, sb_x, ty + 1, ty + inner_h as u16, total, visible, max_scroll.saturating_sub(scroll));
    }
}
