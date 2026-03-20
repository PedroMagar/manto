use std::io::Write;

use crate::{cmd::{CommandEntry, CommandStatus}, ansi, window::Window};

/// Conteúdo fixo da barra de status (antes da área de input).
pub const STATUS_BAR_PREFIX: &str = " Start | .> ";
/// Texto e posição x do botão Start dentro da linha da barra (coluna 0 = │).
pub const STATUS_START: &str = " Start ";
pub const STATUS_START_X: u16 = 1;
/// Coluna X onde começa a área de entrada de comando (após o prefixo completo).
pub const CMD_INPUT_X: u16 = 1 + STATUS_BAR_PREFIX.len() as u16;
/// Número de desktops virtuais exibidos na barra de status.
pub const DESKTOP_COUNT: usize = 4;
/// Largura visual da área de desktop na barra: "| N " × DESKTOP_COUNT = 16 colunas.
pub const DESKTOP_AREA_LEN: u16 = DESKTOP_COUNT as u16 * 4;

pub fn draw_desktop(out: &mut impl Write, theme: u16, w: u16, h: u16, title: &str) {
    match theme {
        1 => {
            ansi::move_to(out, 0, 0);
            write!(out, "└{:─^1$}┘", format!(" {} ", title), w as usize - 2).unwrap();
        }
        2 => {
            ansi::move_to(out, 0, 0);
            write!(out, "┌{:─^1$}┐", format!(" {} ", title), w as usize - 2).unwrap();
            for i in 1..(h - 1) {
                ansi::move_to(out, 0, i);
                write!(out, "│").unwrap();
                ansi::move_to(out, w - 1, i);
                write!(out, "│").unwrap();
            }
        }
        _ => {}
    }
}

/// Desenha a barra de status (3 linhas inferiores).
pub fn draw_status_bar(out: &mut impl Write, w: u16, h: u16, path: &str, panel_open: bool, current_desktop: usize) {
    let inner = (w - 2) as usize;
    let (cl, cr) = if panel_open { ('├', '┤') } else { ('┌', '┐') };
    ansi::move_to(out, 0, h - 3);
    if path.is_empty() {
        write!(out, "{}{:─<width$}{}", cl, "", cr, width = inner).unwrap();
    } else {
        let label = format!("── {} ", path);
        let fill = inner.saturating_sub(label.chars().count());
        write!(out, "{}{}{:─<width$}{}", cl, label, "", cr, width = fill).unwrap();
    }
    ansi::move_to(out, 0, h - 2);
    let prefix_len  = STATUS_BAR_PREFIX.chars().count();
    let desktop_len = DESKTOP_COUNT * 4; // "| N " × 4 = 16 colunas visuais
    let pad = inner.saturating_sub(prefix_len + desktop_len);
    write!(out, "│{}{:<pad$}", STATUS_BAR_PREFIX, "", pad = pad).unwrap();
    for d in 1..=DESKTOP_COUNT {
        write!(out, "|").unwrap();
        if d == current_desktop {
            write!(out, "{} {} {}", ansi::REVERSE, d, ansi::RESET).unwrap();
        } else {
            write!(out, " {} ", d).unwrap();
        }
    }
    write!(out, "│").unwrap();
    ansi::move_to(out, 0, h - 1);
    write!(out, "└{:─<1$}┘", "", inner).unwrap();
}

/// Retorna o índice (1-based) do botão de desktop em (x, y), ou None.
/// Cada botão ocupa 3 colunas visuais ` N `, separadas por `|`.
/// Layout (da esquerda para direita): `| 1 | 2 | 3 | 4 │`
pub fn desktop_at(x: u16, y: u16, w: u16, h: u16) -> Option<usize> {
    if y != h - 2 { return None; }
    let base_x = w.saturating_sub(1 + DESKTOP_AREA_LEN); // coluna do primeiro '|'
    for d in 1..=DESKTOP_COUNT {
        let sep_x = base_x + (d as u16 - 1) * 4;
        let btn_start = sep_x + 1;
        let btn_end   = sep_x + 3;
        if x >= btn_start && x <= btn_end {
            return Some(d);
        }
    }
    None
}

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

/// Desenha uma aba vertical de largura 2. O título rola quando maior que as linhas disponíveis.
pub fn draw_tab(out: &mut impl Write, x: u16, y: u16, height: u16, title: &str, scroll_offset: usize) {
    let content_rows = height.saturating_sub(2) as usize;
    ansi::move_to(out, x, y);
    write!(out, "┌─").unwrap();
    for i in 0..content_rows {
        let ch = tab_content_char(title, content_rows, i, scroll_offset);
        ansi::move_to(out, x, y + 1 + i as u16);
        write!(out, "│{}", ch).unwrap();
    }
    ansi::move_to(out, x, y + height - 1);
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

/// Desenha a scrollbar vertical em (x, top..=bot). Apenas trilha (░) e thumb (█).
pub fn draw_scrollbar(
    out: &mut impl Write,
    x: u16, top: u16, bot: u16,
    total: usize, visible: usize, scroll: usize,
) {
    if total <= visible || bot < top { return; }
    let track_len = (bot - top + 1) as usize;
    let (thumb_pos, thumb_len) = scrollbar_thumb(track_len, total, visible, scroll);
    for row in top..=bot {
        ansi::move_to(out, x, row);
        write!(out, "░").unwrap();
    }
    for i in 0..thumb_len {
        ansi::move_to(out, x, top + thumb_pos as u16 + i as u16);
        write!(out, "█").unwrap();
    }
}

/// Prefixo do prompt de entrada nas janelas de terminal.
pub const TERMINAL_INPUT_PREFIX: &str = " .> ";

/// Desenha o conteúdo de uma janela Terminal sobre o chrome já renderizado.
///
/// Layout interno (de cima para baixo):
///   rows 1 .. h-4  : histórico de comandos (prioridade idêntica ao painel global)
///   row  h-3        : ├─ path ─────────────────────────────────────────────────┤
///   row  h-2        : │ .> input                                               │
///   row  h-1        : └─────────────────────────────────────────────────────────┘  (chrome)
///
/// Requer `win.height >= 5`. Caso contrário é no-op.
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
    let content_h = win.height.saturating_sub(4) as usize;

    // ── Histórico de comandos ─────────────────────────────────────────────────
    let rows = if commands.is_empty() {
        vec![]
    } else {
        let blocks  = build_blocks(commands, inner_w);
        let sr_len  = total_rows(&blocks);
        let scroll  = panel_scroll.min(sr_len.saturating_sub(content_h));

        if scroll == 0 {
            build_priority_rows(&blocks, content_h).0
        } else {
            let clipped = clip_newest(&blocks, scroll);
            let flat    = flatten(&clipped);
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
        write!(out, "{:<width$}", row, width = inner_w).unwrap();
    }
    for i in rows.len()..content_h {
        ansi::move_to(out, lx + 1, ty + 1 + i as u16);
        write!(out, "{:<width$}", "", width = inner_w).unwrap();
    }

    // ── Separador de path ─────────────────────────────────────────────────────
    let path_y = ty + win.height - 3;
    ansi::move_to(out, lx, path_y);
    if path.is_empty() {
        write!(out, "├{:─<1$}┤", "", inner_w).unwrap();
    } else {
        let label = format!("── {} ", path);
        let fill  = inner_w.saturating_sub(label.chars().count());
        write!(out, "├{}{:─<fill$}┤", label, "", fill = fill).unwrap();
    }

    // ── Linha de input (prefixo; conteúdo real é renderizado pelo loop principal) ──
    let input_y   = ty + win.height - 2;
    let prefix_len = TERMINAL_INPUT_PREFIX.chars().count();
    ansi::move_to(out, lx + 1, input_y);
    write!(out, "{}{:<width$}", TERMINAL_INPUT_PREFIX, "", width = inner_w.saturating_sub(prefix_len)).unwrap();
}

// ── Painel de comandos ────────────────────────────────────────────────────────

/// Quebra `line` em fatias de `width` caracteres.
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

/// Um comando com todas as suas linhas pré-quebradas para caber em `width`.
///
/// `outputs` está ordenado do mais antigo (índice 0) ao mais recente (último).
/// `outputs.last()` = resultado final — segunda maior prioridade de exibição.
/// `elision` = indicador `│ ├─ ...` usado quando outputs intermediários são ocultos.
#[derive(Clone)]
struct CommandBlock {
    header:  Vec<String>,
    elision: Vec<String>,
    outputs: Vec<Vec<String>>,
}

fn block_rows(b: &CommandBlock) -> usize {
    b.header.len() + b.outputs.iter().map(|o| o.len()).sum::<usize>()
}

fn total_rows(blocks: &[CommandBlock]) -> usize {
    blocks.iter().map(block_rows).sum()
}

/// Constrói um `CommandBlock` por comando, pré-quebrando as linhas em `width`.
fn build_blocks(commands: &[CommandEntry], width: usize) -> Vec<CommandBlock> {
    commands.iter().map(|entry| {
        let header  = wrap_line(&format!("  ├─┬ {}", entry.command), width);
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
        CommandBlock { header, elision, outputs }
    }).collect()
}

/// Remove `skip` linhas mais recentes da lista de blocos.
///
/// Dentro de cada bloco, os outputs intermediários mais recentes são removidos
/// primeiro; o resultado e o cabeçalho são os últimos a sair.
fn clip_newest(blocks: &[CommandBlock], skip: usize) -> Vec<CommandBlock> {
    if skip == 0 { return blocks.to_vec(); }

    let mut remaining = skip;
    for (i, block) in blocks.iter().enumerate().rev() {
        let n = block_rows(block);
        if remaining >= n {
            remaining -= n;
            continue;
        }
        // Este bloco é parcialmente mantido.
        let keep = n - remaining;
        let mut result = blocks[..i].to_vec();
        if let Some(clipped) = clip_block(block, keep) {
            result.push(clipped);
        }
        return result;
    }
    vec![]
}

/// Mantém apenas as `keep` primeiras linhas de um bloco.
///
/// Prioridade de retenção: cabeçalho → resultado → intermediários mais antigos.
/// Se houver intermediários ocultos e espaço suficiente, injeta `elision` em `outputs`.
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
            // Sacrifica 1 slot para o marcador de elision quando há linhas ocultas.
            let (show, elide) = if hidden > 0 && slots > 0 {
                (slots - 1, true)
            } else {
                (slots.min(internal.len()), false)
            };
            // Mantém os intermediários mais ANTIGOS (do início da lista).
            for row in &internal[..show] {
                outputs.push(vec![row.clone()]);
            }
            if elide { outputs.push(block.elision.clone()); }
            outputs.push(result.clone());
        }
    }

    Some(CommandBlock { header, elision: block.elision.clone(), outputs })
}

/// Uma linha plana com o texto da primeira linha do cabeçalho do bloco ao qual pertence.
/// Permite que o caminho de scroll > 0 sempre pinte o cabeçalho como primeira linha visível.
struct FlatRow {
    text:   String,
    /// Primeira linha do cabeçalho do bloco que contém esta linha.
    header: String,
}

/// Achata `blocks` em `FlatRow`s, preservando a referência ao cabeçalho de cada bloco.
fn flatten(blocks: &[CommandBlock]) -> Vec<FlatRow> {
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

/// Monta as linhas de exibição para scroll=0, aplicando a regra de prioridade:
///
///   1. Cabeçalho do último comando (soberano — sempre exibido primeiro)
///   2. Resultado final/atual do último comando
///   3. Outputs intermediários mais recentes do último comando
///   4. Comandos anteriores (preenchem o espaço restante)
///
/// Retorna `(linhas, algum_oculto)`.
fn build_priority_rows(blocks: &[CommandBlock], area_h: usize) -> (Vec<String>, bool) {
    if area_h == 0 || blocks.is_empty() { return (vec![], false); }

    let mut budget = area_h;
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut any_hidden = false;

    for block in blocks.iter().rev() {
        if budget == 0 { break; }

        // Cabeçalho: sempre a primeira coisa exibida; se não couber, para.
        let h = budget.min(block.header.len());
        if h == 0 { break; }
        budget -= h;

        let mut section: Vec<String> = block.header[..h].to_vec();

        if !block.outputs.is_empty() && budget > 0 {
            if let Some((result, internals)) = block.outputs.split_last() {
                if budget >= result.len() {
                    let internal: Vec<&String> = internals.iter().flatten().collect();
                    let space = budget - result.len();
                    // Mostra elision se há mais intermediários do que espaço e cabe o marcador.
                    let can_elide = internal.len() > space && space > block.elision.len();
                    let elision_cost = if can_elide { block.elision.len() } else { 0 };
                    let show = space.saturating_sub(elision_cost).min(internal.len());
                    let skip = internal.len().saturating_sub(show); // exibe os mais RECENTES

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

/// Desenha o painel de comandos acima da barra de status.
///
/// `scroll=0` aplica a regra de prioridade (cabeçalho soberano).
/// `scroll>0` exibe uma janela deslizante sobre o histórico achatado,
///            garantindo sempre que a primeira linha visível seja um cabeçalho.
///
/// Layout de colunas:
///   col 0       : borda esquerda │
///   cols 1..w-3 : conteúdo  (inner = w-3)
///   col w-2     : scrollbar ░/█
///   col w-1     : │ borda direita
pub fn draw_command_panel(out: &mut impl Write, w: u16, h: u16, path: &str, commands: &[CommandEntry], scroll: usize) {
    if path.is_empty() || commands.is_empty() || w < 5 { return; }
    let inner  = (w - 3) as usize;
    let dash_w = (w - 2) as usize;
    let max_h  = (h as usize * 3 / 4).min(h.saturating_sub(8) as usize);
    if max_h < 3 { return; }

    let path_rows = wrap_line(path, inner);
    let blocks    = build_blocks(commands, inner);
    let sr_len    = total_rows(&blocks);
    if path_rows.is_empty() || sr_len == 0 { return; }

    let path_h  = path_rows.len();
    let panel_h = max_h.min(1 + path_h + sr_len);
    if panel_h <= 1 + path_h { return; }
    let area_h = panel_h - 1 - path_h;

    let max_scroll = sr_len.saturating_sub(area_h);
    let scroll     = scroll.min(max_scroll);
    let top_y      = h.saturating_sub(3 + panel_h as u16);

    // Borda superior
    ansi::move_to(out, 0, top_y);
    write!(out, "┌{:─<1$}┐", "", dash_w).unwrap();
    let mut cur_y = top_y + 1;

    // Path (fixo)
    for row in &path_rows {
        ansi::move_to(out, 0, cur_y);
        write!(out, "│{:<width$} │", row, width = inner).unwrap();
        cur_y += 1;
    }

    // Conteúdo
    let rows = if scroll == 0 {
        build_priority_rows(&blocks, area_h).0
    } else {
        // Janela deslizante: remove `scroll` linhas mais recentes, exibe as restantes.
        let clipped  = clip_newest(&blocks, scroll);
        let flat     = flatten(&clipped);
        let start    = flat.len().saturating_sub(area_h);
        let mut rows: Vec<String> = flat[start..].iter().map(|r| r.text.clone()).collect();
        // Garante que a primeira linha visível seja sempre um cabeçalho.
        if let Some(first) = flat.get(start) {
            if !rows.is_empty() { rows[0] = first.header.clone(); }
        }
        rows
    };

    for row in &rows {
        ansi::move_to(out, 0, cur_y);
        write!(out, "│{:<width$} │", row, width = inner).unwrap();
        cur_y += 1;
    }
    let _ = cur_y;

    if max_scroll > 0 {
        let sb_top = top_y + 1 + path_h as u16;
        let sb_bot = top_y + panel_h as u16 - 1;
        draw_scrollbar(out, w - 2, sb_top, sb_bot, sr_len, area_h, max_scroll - scroll);
    }
}

// ── Testes ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::{CommandEntry, CommandStatus};

    fn timer_cmd(ticks: u32) -> CommandEntry {
        let mut cmd = CommandEntry::new(&format!("timer {}", ticks));
        // Simula os ticks: output_lines já criado com "Ns", adiciona os demais.
        for i in (1..ticks).rev() {
            cmd.output_lines.push(format!("{}s", i));
        }
        // Após todos os ticks normais, o último seria "complete" mas para testes
        // de running deixamos status como Running.
        cmd.status = CommandStatus::Running;
        cmd
    }

    fn simple_cmd(name: &str, output: &str) -> CommandEntry {
        let mut cmd = CommandEntry::new(name);
        // Substitui "command not found" pelo output desejado.
        cmd.output_lines = vec![output.to_string()];
        cmd.status = CommandStatus::Complete;
        cmd
    }

    #[test]
    fn soberano_header_antes_de_ocultar_outputs() {
        // timer 12: header + outputs [12s, 11s, 10s, 9s (running)], area_h=3
        let cmd = timer_cmd(4); // 4 ticks → output_lines = ["4s","3s","2s","1s"]
        let blocks = build_blocks(&[cmd], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 3);
        assert!(hidden);
        assert_eq!(rows[0], "  ├─┬ timer 4");
        // Resultado (última linha) sempre visível.
        assert_eq!(rows.last().unwrap(), "  │ └─ 1s (running)");
    }

    #[test]
    fn resultado_exibido_apos_cabecalho() {
        let cmd = simple_cmd("echo ok", "ok");
        let blocks = build_blocks(&[cmd], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 2);
        assert!(!hidden);
        assert_eq!(rows[0], "  ├─┬ echo ok");
        assert_eq!(rows[1], "  │ └─ ok");
    }

    #[test]
    fn cabecalho_antigo_aparece_quando_ha_espaco() {
        // echo ok (2 linhas) + timer 3 (4 linhas) = 6 linhas no total.
        // Com area_h=5, timer 3 consome 4 linhas (prioridade), echo ok só cabe
        // o cabeçalho (1 linha restante). Nenhum conteúdo é tecnicamente "oculto"
        // pois ambos os blocos estão representados.
        let older  = simple_cmd("echo ok", "ok");
        let latest = timer_cmd(3); // header + [3s,2s,1s]
        let blocks = build_blocks(&[older, latest], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 5);
        assert!(!hidden);
        assert_eq!(rows[0], "  ├─┬ echo ok");   // cabeçalho do antigo
        assert_eq!(rows[1], "  ├─┬ timer 3");   // cabeçalho do recente
        assert_eq!(rows[2], "  │ ├─ 3s");
        assert_eq!(rows[3], "  │ ├─ 2s");
        assert_eq!(rows[4], "  │ └─ 1s (running)");
    }

    #[test]
    fn elision_entre_intermediarios_ocultos_e_resultado() {
        let cmd = timer_cmd(5); // outputs: [5s,4s,3s,2s,1s]
        let blocks = build_blocks(&[cmd], 40);
        let (rows, hidden) = build_priority_rows(&blocks, 4);
        assert!(hidden);
        assert_eq!(rows[0], "  ├─┬ timer 5");
        assert_eq!(rows[1], "  │ ├─ ...");
        assert_eq!(rows[3], "  │ └─ 1s (running)");
    }

    #[test]
    fn clip_mantem_resultado_e_oculta_intermediarios_antigos() {
        let cmd   = timer_cmd(5); // outputs: [5s,4s,3s,2s,1s]
        let block = build_blocks(&[cmd], 40).remove(0);
        // Mantém 4 linhas: header + ??? + resultado
        let clipped = clip_block(&block, 4).unwrap();
        let (rows, hidden) = build_priority_rows(&[clipped], 4);
        assert!(!hidden, "clipped já está dentro do orçamento");
        assert_eq!(rows[0], "  ├─┬ timer 5");
        assert!(rows.iter().any(|r| r == "  │ ├─ ..."));
        assert_eq!(rows.last().unwrap(), "  │ └─ 1s (running)");
    }

    #[test]
    fn clip_newest_remove_mais_recentes_primeiro() {
        let older = simple_cmd("echo ok", "ok");
        let newer = timer_cmd(3); // header + [3s,2s,1s] = 4 linhas
        let blocks = build_blocks(&[older, newer], 40);
        // Remove 2 linhas mais recentes do `newer`.
        let clipped = clip_newest(&blocks, 2);
        let (rows, _) = build_priority_rows(&clipped, 6);
        assert_eq!(rows[0], "  ├─┬ echo ok");
        assert_eq!(rows[1], "  │ └─ ok");
        assert_eq!(rows[2], "  ├─┬ timer 3");
        // O resultado de newer ("1s running") ainda está presente após clip de 2 intermediários.
        assert_eq!(rows[3], "  │ └─ 1s (running)");
    }

    #[test]
    fn scroll_revela_comando_mais_antigo() {
        let older = CommandEntry::new("test"); // header + "command not found"
        let newer = timer_cmd(4); // header + [4s,3s,2s,1s] = 5 linhas
        let blocks = build_blocks(&[older, newer], 40);

        // scroll=2: remove 2 linhas mais recentes do newer
        let clipped = clip_newest(&blocks, 2);
        let flat    = flatten(&clipped);
        let start   = flat.len().saturating_sub(4);
        let mut rows: Vec<String> = flat[start..].iter().map(|r| r.text.clone()).collect();
        if let Some(first) = flat.get(start) {
            if !rows.is_empty() { rows[0] = first.header.clone(); }
        }
        assert_eq!(rows[0], "  ├─┬ test");
        assert_eq!(rows[1], "  ├─┬ timer 4");

        // scroll=3: remove 3 linhas → older fica totalmente visível
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
