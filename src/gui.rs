use std::io::Write;

use crate::{shell::CommandEntry, terminal};

/// Conteúdo fixo da barra de status (antes da área de input).
pub const STATUS_BAR_PREFIX: &str = " Start | .> ";
/// Texto e posição x do botão Start dentro da linha da barra (coluna 0 = │).
/// Inclui os espaços de padding para a área de hover/click.
pub const STATUS_START: &str = " Start ";
pub const STATUS_START_X: u16 = 1; // logo após │
/// Coluna X onde começa a área de entrada de comando (após o prefixo completo).
pub const CMD_INPUT_X: u16 = 1 + STATUS_BAR_PREFIX.len() as u16;

pub fn draw_desktop(out: &mut impl Write, theme: u16, w: u16, h: u16, title: &str) {
    match theme {
        1 => {
            terminal::move_to(out, 0, 0);
            write!(out, "└{:─^1$}┘", format!(" {} ", title), w as usize - 2).unwrap();
        }
        2 => {
            terminal::move_to(out, 0, 0);
            write!(out, "┌{:─^1$}┐", format!(" {} ", title), w as usize - 2).unwrap();

            for i in 1..(h - 1) {
                terminal::move_to(out, 0, i);
                write!(out, "│").unwrap();
                terminal::move_to(out, w - 1, i);
                write!(out, "│").unwrap();
            }
        }
        _ => {}
    }
}

/// Desenha a barra de status (3 linhas inferiores). Deve ser chamada após janelas e painel.
pub fn draw_status_bar(out: &mut impl Write, w: u16, h: u16, path: &str, panel_open: bool) {
    let inner = (w - 2) as usize;
    let (cl, cr) = if panel_open { ('├', '┤') } else { ('┌', '┐') };
    terminal::move_to(out, 0, h - 3);
    if path.is_empty() {
        write!(out, "{}{:─<width$}{}", cl, "", cr, width = inner).unwrap();
    } else {
        let label = format!("── {} ", path);
        let fill = inner.saturating_sub(label.chars().count());
        write!(out, "{}{}{:─<width$}{}", cl, label, "", cr, width = fill).unwrap();
    }

    terminal::move_to(out, 0, h - 2);
    write!(out, "│{:<1$}│", STATUS_BAR_PREFIX, inner).unwrap();

    terminal::move_to(out, 0, h - 1);
    write!(out, "└{:─<1$}┘", "", inner).unwrap();
}

/// Retorna o caractere do conteúdo de uma aba na linha `row` (0-indexed dentro dos content rows).
fn tab_content_char(title: &str, content_rows: usize, row: usize, scroll_offset: usize) -> char {
    let padded = if title.chars().count() > content_rows {
        format!("{}  ", title)
    } else {
        title.to_string()
    };
    let chars: Vec<char> = padded.chars().collect();
    let len = chars.len();
    if len == 0 { ' ' }
    else if len <= content_rows { chars.get(row).copied().unwrap_or(' ') }
    else { chars[(scroll_offset + row) % len] }
}

/// Desenha uma aba vertical de largura 2.
/// O título rola 1 char/segundo quando é maior que as linhas disponíveis.
pub fn draw_tab(out: &mut impl Write, x: u16, y: u16, height: u16, title: &str, scroll_offset: usize) {
    let content_rows = height.saturating_sub(2) as usize;

    terminal::move_to(out, x, y);
    write!(out, "┌─").unwrap();

    for i in 0..content_rows {
        let ch = tab_content_char(title, content_rows, i, scroll_offset);
        terminal::move_to(out, x, y + 1 + i as u16);
        write!(out, "│{}", ch).unwrap();
    }

    terminal::move_to(out, x, y + height - 1);
    write!(out, "└─").unwrap();
}

/// Retorna o caractere visível na posição (x, y) de uma aba.
pub fn tab_char_at(tab_x: u16, tab_y: u16, tab_h: u16, title: &str, x: u16, y: u16, scroll_offset: usize) -> char {
    let content_rows = tab_h.saturating_sub(2) as usize;
    if y == tab_y || y == tab_y + tab_h - 1 {
        return if x == tab_x { if y == tab_y { '┌' } else { '└' } } else { '─' };
    }
    if x == tab_x { return '│'; }
    tab_content_char(title, content_rows, (y - tab_y - 1) as usize, scroll_offset)
}

/// Calcula (thumb_pos, thumb_len) para uma scrollbar.
pub fn scrollbar_thumb(track_len: usize, total: usize, visible: usize, scroll: usize) -> (usize, usize) {
    let thumb_len = (((visible as f32 / total as f32) * track_len as f32).max(1.0) as usize)
        .min(track_len);
    let available = track_len - thumb_len;
    let max_scroll = total - visible;
    let thumb_pos = if max_scroll > 0 { (scroll * available / max_scroll).min(available) } else { 0 };
    (thumb_pos, thumb_len)
}

/// Desenha a scrollbar vertical em (x, top..=bot).
/// `total`   = número total de itens
/// `visible` = número de itens visíveis
/// `scroll`  = posição atual do scroll
/// Sem setas — apenas trilha (░) e thumb (█).
pub fn draw_scrollbar(
    out: &mut impl Write,
    x: u16, top: u16, bot: u16,
    total: usize, visible: usize, scroll: usize,
) {
    if total <= visible || bot < top { return; }

    let track_len = (bot - top + 1) as usize;
    let (thumb_pos, thumb_len) = scrollbar_thumb(track_len, total, visible, scroll);

    for row in top..=bot {
        terminal::move_to(out, x, row);
        write!(out, "░").unwrap();
    }
    for i in 0..thumb_len {
        terminal::move_to(out, x, top + thumb_pos as u16 + i as u16);
        write!(out, "█").unwrap();
    }
}

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

#[allow(dead_code)]
fn expand_rows(lines: &[String], width: usize) -> Vec<String> {
    let mut rows = Vec::new();

    for line in lines {
        rows.extend(wrap_line(line, width));
    }

    rows
}

#[derive(Clone)]
struct WrappedCommandBlock {
    header: Vec<String>,
    elided_output: Vec<String>,
    outputs: Vec<Vec<String>>,
}

#[derive(Clone)]
struct VisibleCommandBlock {
    header: Vec<String>,
    outputs: Vec<Vec<String>>,
}

fn wrap_command_blocks(commands: &[CommandEntry], width: usize) -> Vec<WrappedCommandBlock> {
    commands.iter().map(|entry| {
        let header = wrap_line(&format!("  ├─┬ {}", entry.command), width);
        let elided_output = wrap_line("  │ ├─ ...", width);
        let last_idx = entry.output_lines.len().saturating_sub(1);
        let outputs = entry.output_lines.iter().enumerate().map(|(idx, out_line)| {
            let is_last = idx == last_idx;
            let branch = if is_last { "└─ " } else { "├─ " };
            let suffix = if is_last && !matches!(entry.status, crate::shell::CommandStatus::Complete) {
                " (running)"
            } else {
                ""
            };
            wrap_line(&format!("  │ {}{}{}", branch, out_line, suffix), width)
        }).collect();

        WrappedCommandBlock { header, elided_output, outputs }
    }).collect()
}

#[derive(Clone)]
struct FlatCommandRow {
    text: String,
    block_start: usize,
    is_header: bool,
}

fn flatten_blocks_with_meta(blocks: &[WrappedCommandBlock]) -> Vec<FlatCommandRow> {
    let mut rows = Vec::new();

    for block in blocks {
        let block_start = rows.len();
        for row in &block.header {
            rows.push(FlatCommandRow {
                text: row.clone(),
                block_start,
                is_header: true,
            });
        }
        for output in &block.outputs {
            for row in output {
                rows.push(FlatCommandRow {
                    text: row.clone(),
                    block_start,
                    is_header: false,
                });
            }
        }
    }

    rows
}

fn block_row_count(block: &WrappedCommandBlock) -> usize {
    block.header.len() + block.outputs.iter().map(|rows| rows.len()).sum::<usize>()
}

fn total_block_rows(blocks: &[WrappedCommandBlock]) -> usize {
    blocks.iter().map(block_row_count).sum()
}

fn rows_to_chunks(rows: &[String]) -> Vec<Vec<String>> {
    rows.iter().cloned().map(|row| vec![row]).collect()
}

fn clip_block_from_bottom(block: &WrappedCommandBlock, keep_rows: usize) -> Option<WrappedCommandBlock> {
    let total_rows = block_row_count(block);
    if keep_rows == 0 {
        return None;
    }
    if keep_rows >= total_rows {
        return Some(block.clone());
    }

    let header_keep = keep_rows.min(block.header.len());
    let header = block.header[..header_keep].to_vec();
    if keep_rows <= block.header.len() {
        return Some(WrappedCommandBlock {
            header,
            elided_output: block.elided_output.clone(),
            outputs: Vec::new(),
        });
    }

    let output_keep = keep_rows - block.header.len();
    let mut outputs = Vec::new();

    if let Some((result_rows, internal_chunks)) = block.outputs.split_last() {
        let internal_rows: Vec<String> = internal_chunks.iter().flatten().cloned().collect();

        if output_keep >= result_rows.len() {
            let mut keep_internal = output_keep - result_rows.len();
            let hidden_internal = internal_rows.len().saturating_sub(keep_internal);
            if hidden_internal > 0 && keep_internal > 0 {
                keep_internal -= 1;
            }

            let visible_internal = keep_internal.min(internal_rows.len());
            outputs.extend(rows_to_chunks(&internal_rows[..visible_internal]));
            if hidden_internal > 0 && output_keep > result_rows.len() {
                outputs.push(block.elided_output.clone());
            }
            outputs.push(result_rows.clone());
        }
    }

    Some(WrappedCommandBlock {
        header,
        elided_output: block.elided_output.clone(),
        outputs,
    })
}

fn clip_blocks_from_bottom(blocks: &[WrappedCommandBlock], hidden_rows: usize) -> Vec<WrappedCommandBlock> {
    if hidden_rows == 0 {
        return blocks.to_vec();
    }

    let mut remaining_hidden = hidden_rows;
    let mut kept_rev = Vec::new();

    for block in blocks.iter().rev() {
        let total_rows = block_row_count(block);
        if remaining_hidden >= total_rows {
            remaining_hidden -= total_rows;
            continue;
        }

        let keep_rows = total_rows - remaining_hidden;
        remaining_hidden = 0;
        if let Some(clipped) = clip_block_from_bottom(block, keep_rows) {
            kept_rev.push(clipped);
        }
    }

    kept_rev.reverse();
    kept_rev
}

fn build_auto_rows(blocks: &[WrappedCommandBlock], area_h: usize) -> (Vec<String>, bool) {
    if area_h == 0 || blocks.is_empty() {
        return (Vec::new(), false);
    }

    let mut remaining = area_h;
    let mut visible_rev: Vec<VisibleCommandBlock> = Vec::new();
    let mut any_hidden = false;

    for block in blocks.iter().rev() {
        if remaining == 0 {
            break;
        }

        let header_take = remaining.min(block.header.len());
        if header_take == 0 {
            break;
        }

        let mut visible = VisibleCommandBlock {
            header: block.header[..header_take].to_vec(),
            outputs: Vec::new(),
        };
        remaining -= header_take;

        if !block.outputs.is_empty() && remaining > 0 {
            if let Some((result_rows, internal_chunks)) = block.outputs.split_last() {
                if remaining >= result_rows.len() {
                    let internal_rows: Vec<String> = internal_chunks.iter().flatten().cloned().collect();
                    let internal_total = internal_rows.len();
                    let space_after_result = remaining - result_rows.len();
                    let can_show_elision =
                        internal_total > space_after_result && space_after_result > block.elided_output.len();
                    let reserved_for_elision = if can_show_elision { block.elided_output.len() } else { 0 };
                    let mut output_slots = space_after_result.saturating_sub(reserved_for_elision);
                    let mut outputs_rev = Vec::new();
                    let mut shown_internal = 0;

                    for row in internal_rows.iter().rev() {
                        if output_slots == 0 {
                            break;
                        }
                        outputs_rev.push(vec![row.clone()]);
                        output_slots -= 1;
                        shown_internal += 1;
                    }

                    outputs_rev.reverse();

                    if internal_total > shown_internal {
                        any_hidden = true;
                        if can_show_elision {
                            visible.outputs.push(block.elided_output.clone());
                        }
                    }

                    visible.outputs.extend(outputs_rev);
                    visible.outputs.push(result_rows.clone());
                    remaining -= visible.outputs.iter().map(|rows| rows.len()).sum::<usize>();
                } else {
                    any_hidden = true;
                }
            }
        }

        visible_rev.push(visible);
    }

    let older_commands_hidden = visible_rev.len() < blocks.len();
    if older_commands_hidden {
        any_hidden = true;
    }
    visible_rev.reverse();

    let mut rows = Vec::new();
    for block in visible_rev {
        rows.extend(block.header);
        for output in block.outputs {
            rows.extend(output);
        }
    }

    (rows, any_hidden)
}

/// Desenha o painel de comandos acima da barra de status.
///
/// `scroll` = 0 → conteúdo mais recente; aumenta → conteúdo mais antigo.
/// `last_cmd_rows` = linhas que o último comando ocupa em sr (1 + output_lines.len()).
///
/// O cabeçalho do último comando é SOBERANO em scroll=0:
///   - É sempre a primeira linha exibida.
///   - Quando o output é maior que o espaço disponível, as linhas ANTIGAS
///     do output (não o cabeçalho) são suprimidas silenciosamente.
///
/// Layout de colunas:
///   col 0       : borda esquerda (│ ou ├ nos separadores)
///   cols 1..w-3 : conteúdo  (inner = w-3)
///   col w-2     : scrollbar ░/█ ou espaço
///   col w-1     : │ borda direita
#[allow(dead_code)]
fn draw_command_panel_legacy(out: &mut impl Write, w: u16, h: u16, content_lines: &[String], scroll: usize, last_cmd_rows: usize) {
    if content_lines.is_empty() || w < 5 { return; }
    let inner  = (w - 3) as usize;
    let dash_w = (w - 2) as usize;
    let max_h  = (h as usize * 3 / 4).min(h.saturating_sub(8) as usize);
    if max_h < 3 { return; }

    let all_rows = expand_rows(content_lines, inner);
    if all_rows.is_empty() { return; }

    // all_rows[0] = path (fixo); all_rows[1..] = conteúdo rolável (sr)
    let sr = &all_rows[1..];
    let sr_len = sr.len();

    let panel_h = max_h.min(2 + sr_len); // borda + path + conteúdo
    if panel_h < 3 { return; }
    let area_h = panel_h - 2;

    let max_scroll = sr_len.saturating_sub(area_h);
    let scroll     = scroll.min(max_scroll);

    let top_y = h.saturating_sub(3 + panel_h as u16);

    // Borda superior
    terminal::move_to(out, 0, top_y);
    write!(out, "┌{:─<1$}┐", "", dash_w).unwrap();
    let mut cur_y = top_y + 1;

    // Path (fixo, nunca rola)
    terminal::move_to(out, 0, cur_y);
    write!(out, "│{:<width$} │", &all_rows[0], width = inner).unwrap();
    cur_y += 1;

    // Índice do cabeçalho do último comando dentro de sr
    let header_idx = if last_cmd_rows > 0 { sr_len.saturating_sub(last_cmd_rows) } else { sr_len };

    if scroll == 0 && last_cmd_rows > 0 {
        // ── Modo scroll=0: cabeçalho SOBERANO ──────────────────────────────
        // Mostra: [├─ ... se história oculta] + cabeçalho + output mais recente.
        // Linhas antigas do output são suprimidas (não o cabeçalho).
        let has_history = header_idx > 0;
        let top_sep: usize = if has_history && area_h >= 2 { 1 } else { 0 };

        // Slots para linhas de output após o cabeçalho
        let out_slots = area_h.saturating_sub(1 + top_sep);
        let output = if last_cmd_rows > 1 { &sr[header_idx + 1..] } else { &sr[0..0] };
        let out_start = output.len().saturating_sub(out_slots);
        let out_vis   = &output[out_start..];

        if top_sep > 0 {
            terminal::move_to(out, 0, cur_y);
            write!(out, "├{:<width$} │", "  ├─ ...", width = inner).unwrap();
            cur_y += 1;
        }
        // Cabeçalho soberano
        terminal::move_to(out, 0, cur_y);
        write!(out, "│{:<width$} │", &sr[header_idx], width = inner).unwrap();
        cur_y += 1;
        // Output mais recente
        for row in out_vis {
            terminal::move_to(out, 0, cur_y);
            write!(out, "│{:<width$} │", row, width = inner).unwrap();
            cur_y += 1;
        }
    } else {
        // ── Modo rolagem normal (scroll > 0) ───────────────────────────────
        let content_end = sr_len - scroll;
        let has_below = content_end < sr_len;
        let bot_sep: usize = if has_below && area_h >= 2 { 1 } else { 0 };

        let has_above_natural = content_end.saturating_sub(area_h) > 0;
        let mut top_sep: usize = if has_above_natural && area_h >= 2 { 1 } else { 0 };
        // Suprimir top_sep se ele esconderia o cabeçalho
        if top_sep > 0 {
            let vis_with = area_h.saturating_sub(top_sep + bot_sep);
            if content_end.saturating_sub(vis_with) > header_idx {
                top_sep = 0;
            }
        }

        let vis     = area_h.saturating_sub(top_sep + bot_sep);
        let c_start = content_end.saturating_sub(vis);
        let c_end   = (c_start + vis).min(sr_len);

        if top_sep > 0 {
            terminal::move_to(out, 0, cur_y);
            write!(out, "├{:<width$} │", "  ├─ ...", width = inner).unwrap();
            cur_y += 1;
        }
        for row in &sr[c_start..c_end] {
            terminal::move_to(out, 0, cur_y);
            write!(out, "│{:<width$} │", row, width = inner).unwrap();
            cur_y += 1;
        }
        if bot_sep > 0 {
            terminal::move_to(out, 0, cur_y);
            write!(out, "├{:<width$} │", "  ├─ ...", width = inner).unwrap();
        }
    }
    let _ = cur_y;

    // Scrollbar na coluna w-2, cobrindo toda a área abaixo do path
    if max_scroll > 0 {
        let sb_top = top_y + 2;
        let sb_bot = top_y + panel_h as u16 - 1;
        let inv_scroll = max_scroll - scroll;
        draw_scrollbar(out, w - 2, sb_top, sb_bot, sr_len, area_h, inv_scroll);
    }
}

/// Desenha o painel de comandos acima da barra de status.
///
/// `scroll` = 0 -> prioriza o comando mais recente; aumenta -> mostra histórico.
///
/// Prioridade em `scroll=0`:
///   1. cabeçalho do comando
///   2. resultado final/atual
///   3. resultados intermediários mais recentes
pub fn draw_command_panel(out: &mut impl Write, w: u16, h: u16, path: &str, commands: &[CommandEntry], scroll: usize) {
    if path.is_empty() || commands.is_empty() || w < 5 { return; }
    let inner = (w - 3) as usize;
    let dash_w = (w - 2) as usize;
    let max_h = (h as usize * 3 / 4).min(h.saturating_sub(8) as usize);
    if max_h < 3 { return; }

    let path_rows = wrap_line(path, inner);
    let blocks = wrap_command_blocks(commands, inner);
    let sr_len = total_block_rows(&blocks);
    if path_rows.is_empty() || sr_len == 0 { return; }

    let path_h = path_rows.len();
    // O painel usa a linha superior própria e "fecha" embaixo na barra de status.
    let panel_h = max_h.min(1 + path_h + sr_len);
    if panel_h <= 1 + path_h { return; }
    let area_h = panel_h - 1 - path_h;

    let max_scroll = sr_len.saturating_sub(area_h);
    let scroll = scroll.min(max_scroll);
    let top_y = h.saturating_sub(3 + panel_h as u16);

    terminal::move_to(out, 0, top_y);
    write!(out, "┌{:─<1$}┐", "", dash_w).unwrap();
    let mut cur_y = top_y + 1;

    for row in &path_rows {
        terminal::move_to(out, 0, cur_y);
        write!(out, "│{:<width$} │", row, width = inner).unwrap();
        cur_y += 1;
    }

    let rows = if scroll == 0 {
        build_auto_rows(&blocks, area_h).0
    } else {
        let visible_blocks = clip_blocks_from_bottom(&blocks, scroll);
        let flat_rows = flatten_blocks_with_meta(&visible_blocks);
        let start = flat_rows.len().saturating_sub(area_h);
        let mut rows: Vec<String> = flat_rows[start..]
            .iter()
            .map(|row| row.text.clone())
            .collect();

        if let Some(first_row) = flat_rows.get(start) {
            if !first_row.is_header {
                if let Some(header_row) = flat_rows.get(first_row.block_start) {
                    if !rows.is_empty() {
                        rows[0] = header_row.text.clone();
                    }
                }
            }
        }

        rows
    };

    for row in &rows {
        terminal::move_to(out, 0, cur_y);
        write!(out, "│{:<width$} │", row, width = inner).unwrap();
        cur_y += 1;
    }

    let _ = cur_y;

    if max_scroll > 0 {
        let sb_top = top_y + 1 + path_h as u16;
        let sb_bot = top_y + panel_h as u16 - 1;
        let inv_scroll = max_scroll - scroll;
        draw_scrollbar(out, w - 2, sb_top, sb_bot, sr_len, area_h, inv_scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::{build_auto_rows, clip_block_from_bottom, clip_blocks_from_bottom, flatten_blocks_with_meta, wrap_command_blocks};
    use crate::shell::{CommandEntry, CommandStatus};

    #[test]
    fn keeps_latest_command_header_before_hiding_outputs() {
        let mut cmd = CommandEntry::new("timer 12");
        cmd.output_lines = vec![
            "12s".to_string(),
            "11s".to_string(),
            "10s".to_string(),
            "9s".to_string(),
        ];
        cmd.status = CommandStatus::Running;

        let blocks = wrap_command_blocks(&[cmd], 40);
        let (rows, older_hidden) = build_auto_rows(&blocks, 3);

        assert!(older_hidden);
        assert_eq!(rows[0], "  ├─┬ timer 12");
        assert_eq!(rows[1], "  │ ├─ 10s");
        assert_eq!(rows[2], "  │ └─ 9s (running)");
    }

    #[test]
    fn keeps_previous_command_after_latest_block_priority_is_satisfied() {
        let mut older = CommandEntry::new("echo ok");
        older.output_lines = vec!["ok".to_string()];
        older.status = CommandStatus::Complete;

        let mut latest = CommandEntry::new("timer 3");
        latest.output_lines = vec!["3s".to_string(), "2s".to_string(), "1s".to_string()];
        latest.status = CommandStatus::Running;

        let blocks = wrap_command_blocks(&[older, latest], 40);
        let (rows, older_hidden) = build_auto_rows(&blocks, 5);

        assert!(!older_hidden);
        assert_eq!(rows[0], "  ├─┬ echo ok");
        assert_eq!(rows[1], "  ├─┬ timer 3");
        assert_eq!(rows[2], "  │ ├─ 3s");
        assert_eq!(rows[3], "  │ ├─ 2s");
        assert_eq!(rows[4], "  │ └─ 1s (running)");
    }

    #[test]
    fn preserves_latest_command_header_when_history_indicator_is_needed() {
        let mut older = CommandEntry::new("echo ok");
        older.output_lines = vec!["ok".to_string()];
        older.status = CommandStatus::Complete;

        let mut latest = CommandEntry::new("timer 12");
        latest.output_lines = vec![
            "12s".to_string(),
            "11s".to_string(),
            "10s".to_string(),
            "9s".to_string(),
        ];
        latest.status = CommandStatus::Running;

        let blocks = wrap_command_blocks(&[older, latest], 40);
        let (rows_full, older_hidden) = build_auto_rows(&blocks, 3);

        assert!(older_hidden);
        assert_eq!(rows_full[0], "  ├─┬ timer 12");

        let rows_with_sep = build_auto_rows(&blocks, 2).0;
        assert_eq!(rows_with_sep[0], "  ├─┬ timer 12");
        assert_eq!(rows_with_sep[1], "  │ └─ 9s (running)");
    }

    #[test]
    fn elides_hidden_output_inside_command_tree() {
        let mut latest = CommandEntry::new("timer 12");
        latest.output_lines = vec![
            "12s".to_string(),
            "11s".to_string(),
            "10s".to_string(),
            "9s".to_string(),
            "8s".to_string(),
        ];
        latest.status = CommandStatus::Running;

        let blocks = wrap_command_blocks(&[latest], 40);
        let (rows, hidden) = build_auto_rows(&blocks, 4);

        assert!(hidden);
        assert_eq!(rows[0], "  ├─┬ timer 12");
        assert_eq!(rows[1], "  │ ├─ ...");
        assert_eq!(rows[2], "  │ ├─ 9s");
        assert_eq!(rows[3], "  │ └─ 8s (running)");
    }

    #[test]
    fn clip_from_bottom_hides_recent_internal_before_result() {
        let mut latest = CommandEntry::new("timer 12");
        latest.output_lines = vec![
            "12s".to_string(),
            "11s".to_string(),
            "10s".to_string(),
            "9s".to_string(),
            "8s".to_string(),
        ];
        latest.status = CommandStatus::Running;

        let block = wrap_command_blocks(&[latest], 40).remove(0);
        let clipped = clip_block_from_bottom(&block, 4).unwrap();
        let (rows, hidden) = build_auto_rows(&[clipped], 4);

        assert!(!hidden);
        assert_eq!(rows[0], "  ├─┬ timer 12");
        assert!(rows.iter().any(|row| row == "  │ ├─ ..."));
        assert_eq!(rows.last().unwrap(), "  │ └─ 8s (running)");
    }

    #[test]
    fn clip_blocks_from_bottom_removes_newer_commands_before_older_ones() {
        let mut older = CommandEntry::new("echo ok");
        older.output_lines = vec!["ok".to_string()];
        older.status = CommandStatus::Complete;

        let mut newer = CommandEntry::new("timer 3");
        newer.output_lines = vec!["3s".to_string(), "2s".to_string(), "1s".to_string()];
        newer.status = CommandStatus::Running;

        let clipped = clip_blocks_from_bottom(&wrap_command_blocks(&[older, newer], 40), 2);
        let (rows, _) = build_auto_rows(&clipped, 6);

        assert_eq!(rows[0], "  ├─┬ echo ok");
        assert_eq!(rows[1], "  │ └─ ok");
        assert_eq!(rows[2], "  ├─┬ timer 3");
    }

    #[test]
    fn restores_single_command_not_found_result() {
        let cmd = CommandEntry::new("missing");
        let blocks = wrap_command_blocks(&[cmd], 40);

        let hidden = clip_blocks_from_bottom(&blocks, 1);
        let (hidden_rows, _) = build_auto_rows(&hidden, 2);
        assert_eq!(hidden_rows, vec!["  ├─┬ missing".to_string()]);

        let (restored_rows, restored_hidden) = build_auto_rows(&blocks, 2);
        assert!(!restored_hidden);
        assert_eq!(restored_rows[0], "  ├─┬ missing");
        assert_eq!(restored_rows[1], "  │ └─ command not found");
    }

    #[test]
    fn older_single_result_reappears_when_scrolling_up() {
        let older = CommandEntry::new("test");

        let mut newer = CommandEntry::new("timer 12");
        newer.output_lines = vec![
            "12s".to_string(),
            "11s".to_string(),
            "10s".to_string(),
            "9s".to_string(),
        ];
        newer.status = CommandStatus::Running;

        let wrapped = wrap_command_blocks(&[older, newer], 40);
        let visible_blocks = clip_blocks_from_bottom(&wrapped, 2);
        let flat_rows = flatten_blocks_with_meta(&visible_blocks);
        let start = flat_rows.len().saturating_sub(4);
        let mut rows: Vec<String> = flat_rows[start..].iter().map(|row| row.text.clone()).collect();
        if !flat_rows[start].is_header {
            rows[0] = flat_rows[flat_rows[start].block_start].text.clone();
        }

        assert_eq!(rows[0], "  ├─┬ test");
        assert_eq!(rows[1], "  ├─┬ timer 12");
        assert_eq!(rows[2], "  │ ├─ ...");
        assert_eq!(rows[3], "  │ └─ 9s (running)");

        let visible_blocks_more = clip_blocks_from_bottom(&wrapped, 3);
        let flat_rows_more = flatten_blocks_with_meta(&visible_blocks_more);
        let start_more = flat_rows_more.len().saturating_sub(4);
        let mut rows_more: Vec<String> = flat_rows_more[start_more..].iter().map(|row| row.text.clone()).collect();
        if !flat_rows_more[start_more].is_header {
            rows_more[0] = flat_rows_more[flat_rows_more[start_more].block_start].text.clone();
        }

        assert_eq!(rows_more[0], "  ├─┬ test");
        assert_eq!(rows_more[1], "  │ └─ command not found");
    }
}
