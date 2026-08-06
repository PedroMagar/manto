use std::io::Write;

use super::ansi;
use super::terminal_view::{slice_line, terminal_content_width};
use super::draw_scrollbar;
use crate::cmd::{CommandEntry, CommandStatus};

/// Break `line` into slices of `width` characters.
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut rem = line;
    loop {
        if rem.chars().count() <= width {
            rows.push(rem.to_string());
            break;
        }
        let cut = rem.char_indices().nth(width).map(|(i, _)| i).unwrap_or(rem.len());
        rows.push(rem[..cut].to_string());
        rem = &rem[cut..];
    }
    rows
}

/// One command with all its lines pre-wrapped to fit `width`.
///
/// `header` may start with a directory row when the command begins in a cwd
/// different from the previous block.
///
/// `outputs` is ordered from oldest (index 0) to newest (last).
/// `outputs.last()` = final result, the second-highest display priority.
/// `elision` = the `│ ├─ ...` marker used when intermediate outputs are hidden.
#[derive(Clone)]
pub(super) struct CommandBlock {
    pub(super) header:  Vec<String>,
    pub(super) elision: Vec<String>,
    pub(super) outputs: Vec<Vec<String>>,
}

pub(super) fn block_rows(b: &CommandBlock) -> usize {
    b.header.len() + b.outputs.iter().map(|o| o.len()).sum::<usize>()
}

pub(super) fn total_rows(blocks: &[CommandBlock]) -> usize {
    blocks.iter().map(block_rows).sum()
}

/// Build one `CommandBlock` per command, pre-wrapping lines to `width`.
pub(super) fn build_blocks(commands: &[CommandEntry], width: usize) -> Vec<CommandBlock> {
    let mut blocks = Vec::with_capacity(commands.len());
    let mut last_cwd: Option<&str> = None;

    for entry in commands {
        let mut header = Vec::new();
        if !entry.cwd.is_empty() && last_cwd != Some(entry.cwd.as_str()) {
            header.extend(wrap_line(&entry.cwd, width));
        }
        header.extend(wrap_line(&format!("  ├─┬ {}", entry.command), width));

        let elision = wrap_line("  │ ├─ ...", width);
        let last_idx = entry.output_lines.len().saturating_sub(1);
        let outputs = entry.output_lines.iter().enumerate().map(|(i, line)| {
            let branch = if i == last_idx { "└─ " } else { "├─ " };
            let suffix = if i == last_idx && !matches!(entry.status, CommandStatus::Complete) {
                " (running)"
            } else {
                ""
            };
            wrap_line(&format!("  │ {}{}{}", branch, line, suffix), width)
        }).collect();

        blocks.push(CommandBlock { header, elision, outputs });
        last_cwd = if entry.cwd.is_empty() {
            last_cwd
        } else {
            Some(entry.cwd.as_str())
        };
    }

    blocks
}

/// Remove the `skip` most recent rows from the block list.
///
/// Within each block the newest intermediate outputs are removed first; the
/// result and the header are the last to go.
pub(super) fn clip_newest(blocks: &[CommandBlock], skip: usize) -> Vec<CommandBlock> {
    if skip == 0 { return blocks.to_vec(); }

    let mut remaining = skip;
    for (i, block) in blocks.iter().enumerate().rev() {
        let n = block_rows(block);
        if remaining >= n {
            remaining -= n;
            continue;
        }
        // This block is partially kept.
        let keep = n - remaining;
        let mut result = blocks[..i].to_vec();
        if let Some(clipped) = clip_block(block, keep) {
            result.push(clipped);
        }
        return result;
    }
    vec![]
}

/// Keep only the first `keep` rows of a block.
///
/// Retention priority: header -> result -> oldest intermediates.
/// If there are hidden intermediates and enough space, `elision` is injected
/// into `outputs`.
fn clip_block(block: &CommandBlock, keep: usize) -> Option<CommandBlock> {
    if keep == 0 { return None; }
    if keep >= block_rows(block) { return Some(block.clone()); }

    let h = block.header.len().min(keep);
    let header = block.header[..h].to_vec();
    if h == keep {
        return Some(CommandBlock { header, elision: block.elision.clone(), outputs: vec![] });
    }

    let out_budget = keep - h;
    let mut outputs = vec![];

    if let Some((result, internals)) = block.outputs.split_last() {
        if out_budget >= result.len() {
            let internal: Vec<String> = internals.iter().flatten().cloned().collect();
            let slots  = out_budget - result.len();
            let hidden = internal.len().saturating_sub(slots);
            // Sacrifice one slot for the elision marker when rows are hidden.
            let (show, elide) = if hidden > 0 && slots > 0 {
                (slots - 1, true)
            } else {
                (slots.min(internal.len()), false)
            };
            // Keep the OLDEST intermediates (from the start of the list).
            for row in &internal[..show] {
                outputs.push(vec![row.clone()]);
            }
            if elide { outputs.push(block.elision.clone()); }
            outputs.push(result.clone());
        }
    }

    Some(CommandBlock { header, elision: block.elision.clone(), outputs })
}

/// A flat row carrying the first header line of the block it belongs to.
/// Lets the scroll > 0 path always paint the header as the first visible row.
pub(super) struct FlatRow {
    pub(super) text:   String,
    /// First line of the header of the block containing this row.
    pub(super) header: String,
}

/// Flatten `blocks` into `FlatRow`s, preserving each block's header reference.
pub(super) fn flatten(blocks: &[CommandBlock]) -> Vec<FlatRow> {
    let mut rows = Vec::new();
    for block in blocks {
        let header = block.header.first().cloned().unwrap_or_default();
        for row in &block.header {
            rows.push(FlatRow { text: row.clone(), header: header.clone() });
        }
        for output in &block.outputs {
            for row in output {
                rows.push(FlatRow { text: row.clone(), header: header.clone() });
            }
        }
    }
    rows
}

/// Build the display rows for scroll=0, applying the priority rule:
///
///   1. Header of the last command (sovereign, always shown first)
///   2. Final/current result of the last command
///   3. Newest intermediate outputs of the last command
///   4. Previous commands (fill the remaining space)
///
/// Returns `(rows, any_hidden)`.
pub(super) fn build_priority_rows(blocks: &[CommandBlock], area_h: usize) -> (Vec<String>, bool) {
    if area_h == 0 || blocks.is_empty() { return (vec![], false); }

    let mut budget = area_h;
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut any_hidden = false;

    for block in blocks.iter().rev() {
        if budget == 0 { break; }

        // Header: always the first thing shown; stop if it does not fit.
        let h = budget.min(block.header.len());
        if h == 0 { break; }
        budget -= h;

        let mut section: Vec<String> = block.header[..h].to_vec();

        if !block.outputs.is_empty() && budget > 0 {
            if let Some((result, internals)) = block.outputs.split_last() {
                if budget >= result.len() {
                    let internal: Vec<&String> = internals.iter().flatten().collect();
                    let space = budget - result.len();
                    // Show the elision marker if there are more intermediates
                    // than space and the marker itself fits.
                    let can_elide = internal.len() > space && space > block.elision.len();
                    let elision_cost = if can_elide { block.elision.len() } else { 0 };
                    let show = space.saturating_sub(elision_cost).min(internal.len());
                    let skip = internal.len().saturating_sub(show); // show the most RECENT

                    if internal.len() > show { any_hidden = true; }
                    if can_elide { section.extend(block.elision.iter().cloned()); }
                    section.extend(internal[skip..].iter().map(|s| (*s).clone()));
                    section.extend(result.iter().cloned());
                    budget -= section.len() - h;
                } else {
                    any_hidden = true;
                }
            }
        }

        sections.push(section);
    }

    if sections.len() < blocks.len() { any_hidden = true; }
    sections.reverse();
    (sections.into_iter().flatten().collect(), any_hidden)
}

/// Draw the command panel above the status bar.
///
/// `scroll=0` applies the priority rule (sovereign header).
/// `scroll>0` shows a sliding window over the flattened history, always
///            ensuring the first visible row is a header.
///
/// Column layout:
///   col 0       : left border │
///   cols 1..w-3 : content  (inner = w-3)
///   col w-2     : scrollbar ░/█
///   col w-1     : │ right border
pub fn draw_command_panel(out: &mut impl Write, w: u16, h: u16, path: &str, commands: &[CommandEntry], scroll: usize) {
    if path.is_empty() || commands.is_empty() || w < 5 { return; }
    let inner  = (w - 3) as usize;
    let dash_w = (w - 2) as usize;
    let max_h  = (h as usize * 3 / 4).min(h.saturating_sub(8) as usize);
    if max_h < 3 { return; }

    let content_w = terminal_content_width(path, commands).max(inner);
    let path_rows = vec![path.to_string()];
    let blocks    = build_blocks(commands, content_w);
    let sr_len    = total_rows(&blocks);
    if path_rows.is_empty() || sr_len == 0 { return; }

    let path_h  = path_rows.len();
    let panel_h = max_h.min(1 + path_h + sr_len);
    if panel_h <= 1 + path_h { return; }
    let area_h = panel_h - 1 - path_h;

    let max_scroll = sr_len.saturating_sub(area_h);
    let scroll     = scroll.min(max_scroll);
    let top_y      = h.saturating_sub(3 + panel_h as u16);

    // Top border
    ansi::move_to(out, 0, top_y);
    write!(out, "┌{:─<1$}┐", "", dash_w).unwrap();
    let mut cur_y = top_y + 1;

    // Path (fixed)
    for row in &path_rows {
        ansi::move_to(out, 0, cur_y);
        let display = slice_line(row, 0, inner);
        write!(out, "│{:<width$} │", display, width = inner).unwrap();
        cur_y += 1;
    }

    // Content
    let rows = if scroll == 0 {
        build_priority_rows(&blocks, area_h).0
    } else {
        // Sliding window: remove the `scroll` most recent rows, show the rest.
        let clipped  = clip_newest(&blocks, scroll);
        let flat     = flatten(&clipped);
        let start    = flat.len().saturating_sub(area_h);
        let mut rows: Vec<String> = flat[start..].iter().map(|r| r.text.clone()).collect();
        // Ensure the first visible row is always a header.
        if let Some(first) = flat.get(start) {
            if !rows.is_empty() { rows[0] = first.header.clone(); }
        }
        rows
    };

    for row in &rows {
        ansi::move_to(out, 0, cur_y);
        let display = slice_line(row, 0, inner);
        write!(out, "│{:<width$} │", display, width = inner).unwrap();
        cur_y += 1;
    }
    let _ = cur_y;

    if max_scroll > 0 {
        let sb_top = top_y + 1 + path_h as u16;
        let sb_bot = top_y + panel_h as u16 - 1;
        draw_scrollbar(out, w - 2, sb_top, sb_bot, sr_len, area_h, max_scroll - scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::{CommandEntry, CommandStatus};

    fn running_cmd(name: &str, ticks: u32) -> CommandEntry {
        let mut outputs = Vec::new();
        for i in (1..=ticks).rev() {
            outputs.push(format!("{}s", i));
        }
        let refs: Vec<&str> = outputs.iter().map(String::as_str).collect();
        CommandEntry::fixture(name, &refs, CommandStatus::Running)
    }

    fn simple_cmd(name: &str, output: &str) -> CommandEntry {
        CommandEntry::fixture(name, &[output], CommandStatus::Complete)
    }

    #[test]
    fn header_stays_visible_before_hiding_outputs() {
        let cmd = running_cmd("count 4", 4);
        let blocks = build_blocks(&[cmd], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 3);
        assert!(hidden);
        assert_eq!(rows[0], "  ├─┬ count 4");
        // The result (last line) is always visible.
        assert_eq!(rows.last().unwrap(), "  │ └─ 1s (running)");
    }

    #[test]
    fn result_shown_after_header() {
        let cmd = simple_cmd("echo ok", "ok");
        let blocks = build_blocks(&[cmd], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 2);
        assert!(!hidden);
        assert_eq!(rows[0], "  ├─┬ echo ok");
        assert_eq!(rows[1], "  │ └─ ok");
    }

    #[test]
    fn older_header_appears_when_space_allows() {
        // echo ok (2 rows) + count 3 (4 rows) = 6 rows total.
        // With area_h=5, count 3 consumes 4 rows (priority) and echo ok only
        // fits its header (1 remaining row). No content is technically hidden
        // since both blocks are represented.
        let older  = simple_cmd("echo ok", "ok");
        let latest = running_cmd("count 3", 3);
        let blocks = build_blocks(&[older, latest], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 5);
        assert!(!hidden);
        assert_eq!(rows[0], "  ├─┬ echo ok");   // older header
        assert_eq!(rows[1], "  ├─┬ count 3");   // newest header
        assert_eq!(rows[2], "  │ ├─ 3s");
        assert_eq!(rows[3], "  │ ├─ 2s");
        assert_eq!(rows[4], "  │ └─ 1s (running)");
    }

    #[test]
    fn elision_between_hidden_intermediates_and_result() {
        let cmd = running_cmd("count 5", 5);
        let blocks = build_blocks(&[cmd], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 4);
        assert!(hidden);
        assert_eq!(rows[0], "  ├─┬ count 5");
        assert_eq!(rows[1], "  │ ├─ ...");
        assert_eq!(rows[3], "  │ └─ 1s (running)");
    }

    #[test]
    fn clip_keeps_result_and_hides_old_intermediates() {
        let cmd   = running_cmd("count 5", 5);
        let block = build_blocks(&[cmd], 40).remove(0);
        // Keep 4 rows: header + ??? + result
        let clipped = clip_block(&block, 4).unwrap();
        let (rows, hidden) = build_priority_rows(&[clipped], 4);
        assert!(!hidden, "clipped is already within budget");
        assert_eq!(rows[0], "  ├─┬ count 5");
        assert!(rows.iter().any(|r| r == "  │ ├─ ..."));
        assert_eq!(rows.last().unwrap(), "  │ └─ 1s (running)");
    }

    #[test]
    fn clip_newest_removes_most_recent_first() {
        let older = simple_cmd("echo ok", "ok");
        let newer = running_cmd("count 3", 3);
        let blocks = build_blocks(&[older, newer], 40);
        // Remove the 2 most recent rows from `newer`.
        let clipped = clip_newest(&blocks, 2);
        let (rows, _) = build_priority_rows(&clipped, 6);
        assert_eq!(rows[0], "  ├─┬ echo ok");
        assert_eq!(rows[1], "  │ └─ ok");
        assert_eq!(rows[2], "  ├─┬ count 3");
        // The result of newer ("1s running") is still present after clipping 2 intermediates.
        assert_eq!(rows[3], "  │ └─ 1s (running)");
    }

    #[test]
    fn scroll_reveals_older_command() {
        let older = CommandEntry::fixture("test", &["command not found"], CommandStatus::Complete);
        let newer = running_cmd("count 4", 4);
        let blocks = build_blocks(&[older, newer], 40);

        // scroll=2: remove the 2 most recent rows from newer
        let clipped = clip_newest(&blocks, 2);
        let flat    = flatten(&clipped);
        let start   = flat.len().saturating_sub(4);
        let mut rows: Vec<String> = flat[start..].iter().map(|r| r.text.clone()).collect();
        if let Some(first) = flat.get(start) {
            if !rows.is_empty() { rows[0] = first.header.clone(); }
        }
        assert_eq!(rows[0], "  ├─┬ test");
        assert_eq!(rows[1], "  ├─┬ count 4");

        // scroll=3: remove 3 rows -> older becomes fully visible
        let clipped2 = clip_newest(&blocks, 3);
        let flat2    = flatten(&clipped2);
        let start2   = flat2.len().saturating_sub(4);
        let mut rows2: Vec<String> = flat2[start2..].iter().map(|r| r.text.clone()).collect();
        if let Some(first) = flat2.get(start2) {
            if !rows2.is_empty() { rows2[0] = first.header.clone(); }
        }
        assert_eq!(rows2[0], "  ├─┬ test");
        assert_eq!(rows2[1], "  │ └─ command not found");
    }
}
