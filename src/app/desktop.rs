// Desktop session state: every piece of mutable state of the running
// environment, plus the input handling that drives it.

use std::io::Write;
use std::time::Duration;

use super::terminal::{interactive_command, is_interactive_app, is_repl_exit, split_interactive_flag};
use super::Application;
use crate::cmd::{tick_all, CommandEntry};
use crate::input::{self, History};
use crate::os::{self, Clock, Key};
use crate::ui::pointer::Pointer;
use crate::ui::{compute_render_state, desktop_at, render, CMD_INPUT_X, STATUS_BAR_PREFIX,
                STATUS_START, STATUS_START_X, TERMINAL_INPUT_PREFIX};
use crate::wm::{self, apply_resize_edit, bring_window_to_front, close_active_window,
                enter_active_resize_mode, focus_relative_window, max_tab_scroll,
                minimize_active_window, move_active_window_to_desktop, normalize_host_path,
                place_pointer_on_terminal_input, push_shell_command, resolve_snap_region,
                snap_active_window, spawn_interactive_terminal, spawn_terminal_window,
                split_active_terminal_window, sync_terminal_window_metrics, tab_layout,
                toggle_active_maximize, toggle_start_menu, topmost_window_at, Mode, ResizeEditState};

pub struct Desktop {
    pub mode: Mode,
    pub scroll_offset: usize,
    pub tab_scroll: usize,
    pub panel_scroll: usize,
    pub current_desktop: usize,
    pub next_terminal_id: usize,
    pub last_space_time: Option<Clock>,
    pub current_path: String,
    pub cmd_input: String,
    pub cmd_cursor: usize,
    pub history_index: Option<usize>,
    pub history_draft: Option<String>,
    pub last_size: (u16, u16),
    pub pointer: Pointer,
    pub applications: Vec<Application>,
    pub history: History,
    pub commands: Vec<CommandEntry>,
    /// Set when the user requests quit (Ctrl+Delete).
    pub quit: bool,
}

impl Desktop {
    pub fn new() -> Self {
        let current_path = std::env::current_dir()
            .map(|path| normalize_host_path(&path))
            .unwrap_or_else(|_| ".".to_string());
        let last_size = os::size();
        let pointer = Pointer::new(1 + STATUS_BAR_PREFIX.len() as u16, last_size.1 - 2);

        let mut applications = Vec::new();
        sync_terminal_window_metrics(&mut applications);

        let history = History::new();
        let loaded_history = history.load(1000);
        let commands: Vec<CommandEntry> = if loaded_history.is_empty() {
            Vec::new()
        } else {
            let cwd = current_path.clone();
            loaded_history.iter().map(|line| {
                CommandEntry::completed(line, &cwd, vec![line.clone()])
            }).collect()
        };

        Self {
            mode: Mode::Normal,
            scroll_offset: 0,
            tab_scroll: 0,
            panel_scroll: 0,
            current_desktop: 1,
            next_terminal_id: 1,
            last_space_time: None,
            current_path,
            cmd_input: String::new(),
            cmd_cursor: 0,
            history_index: None,
            history_draft: None,
            last_size,
            pointer,
            applications,
            history,
            commands,
            quit: false,
        }
    }

    /// Sync window metrics and draw a full frame.
    pub fn draw<W: Write>(&mut self, out: &mut W) {
        sync_terminal_window_metrics(&mut self.applications);
        let (preview, cursor) = compute_render_state(&self.mode, &self.applications, &self.pointer);
        let in_shell = matches!(self.mode, Mode::Typing);
        let shell_path = if in_shell { self.current_path.as_str() } else { "" };
        let focused_term = if let Mode::TerminalFocus { app_idx } = &self.mode {
            self.applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str(), t.input_cursor))
        } else { None };
        render(
            out,
            &self.applications,
            preview,
            cursor,
            self.last_size.0,
            self.last_size.1,
            &self.pointer,
            self.scroll_offset,
            self.tab_scroll,
            shell_path,
            if in_shell { Some((self.cmd_input.as_str(), self.cmd_cursor)) } else { None },
            if in_shell { &self.commands } else { &[] },
            self.panel_scroll,
            self.current_desktop,
            focused_term,
        );
    }

    /// Read and handle one key event. Returns true when a redraw is needed.
    pub fn step_input(&mut self) -> bool {
        let key = os::read_key();
        if matches!(key, Key::CtrlDelete) {
            self.quit = true;
            return false;
        }

        let prev = (self.pointer.x, self.pointer.y);
        let mode_changed = self.handle_key(key);

        // Keep the pointer off the scrollbar column unless there is a tab to
        // scroll.
        if matches!(&self.mode, Mode::Normal) {
            let sb_x = self.last_size.0.saturating_sub(1);
            if self.pointer.x == sb_x {
                let minimized_count = self.applications.iter()
                    .filter(|a| a.on_desktop(self.current_desktop) && a.is_minimized())
                    .count();
                let tab_count = tab_layout(&self.applications, self.current_desktop, self.last_size.1, self.tab_scroll).len();
                if minimized_count <= tab_count {
                    self.pointer.x = sb_x.saturating_sub(1);
                } else {
                    let sb_top = 1u16;
                    let sb_bot = self.last_size.1.saturating_sub(4);
                    self.pointer.y = self.pointer.y.max(sb_top).min(sb_bot);
                }
            }
        }

        // While moving, the window follows the pointer.
        if let Mode::Moving { app_idx, offset_x } = &self.mode {
            let (app_idx, offset_x) = (*app_idx, *offset_x);
            if let Some(win) = self.applications[app_idx].window_mut() {
                win.position_x = self.pointer.x.saturating_sub(offset_x);
                win.position_y = self.pointer.y;
            }
        }

        let moved = (self.pointer.x, self.pointer.y) != prev;
        moved || mode_changed
    }

    /// Advance running commands and shell sessions. Returns true when
    /// anything changed (a redraw is needed).
    pub fn tick(&mut self) -> bool {
        tick_all(&mut self.commands)
            || self.applications.iter_mut().any(|a| {
                a.terminal.as_mut().map_or(false, |t| t.tick())
            })
    }

    /// Per-second housekeeping: tab title marquee, host terminal resize, and
    /// hovered-tab scrolling. Returns true when a redraw is needed.
    pub fn on_second_tick(&mut self) -> bool {
        self.scroll_offset = self.scroll_offset.wrapping_add(1);
        let new_size = os::size();
        let size_changed = new_size != self.last_size;
        if size_changed {
            self.pointer.y = new_size.1 - (self.last_size.1 - self.pointer.y);
            self.last_size = new_size;
            self.pointer.clamp_to_bounds(self.last_size.0, self.last_size.1);
            self.tab_scroll = self.tab_scroll.min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
        }
        let tab_x = self.last_size.0.saturating_sub(3);
        let needs_scroll = tab_layout(&self.applications, self.current_desktop, self.last_size.1, self.tab_scroll)
            .iter()
            .any(|&(idx, tab_y, tab_h)| {
                let is_hovered = self.pointer.x >= tab_x
                    && self.pointer.y >= tab_y
                    && self.pointer.y < tab_y + tab_h;
                is_hovered
                    && self.applications[idx].title.chars().count() > tab_h.saturating_sub(2) as usize
            });
        size_changed || needs_scroll
    }

    fn mode_app_idx(&self) -> Option<usize> {
        match &self.mode {
            Mode::Moving { app_idx, .. }
            | Mode::Resizing { app_idx, .. }
            | Mode::TerminalFocus { app_idx } => Some(*app_idx),
            _ => None,
        }
    }

    fn handle_key(&mut self, key: Key) -> bool {
        // Interactive terminals forward every key raw to the session (except
        // Esc/End -> desktop, Ctrl+Delete -> quit handled in step_input).
        if let Mode::TerminalFocus { app_idx } = &self.mode {
            let app_idx = *app_idx;
            if self.applications.get(app_idx)
                .and_then(|a| a.terminal.as_ref())
                .map_or(false, |t| t.interactive)
            {
                return self.key_interactive(app_idx, key);
            }
        }

        let mut mode_changed = false;

        match key {
            Key::Ctrl1 => {
                if move_active_window_to_desktop(&mut self.applications, &mut self.mode, &mut self.current_desktop, 1, self.last_size.1, &mut self.tab_scroll) {
                    mode_changed = true;
                }
            }
            Key::Ctrl2 => {
                if move_active_window_to_desktop(&mut self.applications, &mut self.mode, &mut self.current_desktop, 2, self.last_size.1, &mut self.tab_scroll) {
                    mode_changed = true;
                }
            }
            Key::Ctrl3 => {
                if move_active_window_to_desktop(&mut self.applications, &mut self.mode, &mut self.current_desktop, 3, self.last_size.1, &mut self.tab_scroll) {
                    mode_changed = true;
                }
            }
            Key::Ctrl4 => {
                if move_active_window_to_desktop(&mut self.applications, &mut self.mode, &mut self.current_desktop, 4, self.last_size.1, &mut self.tab_scroll) {
                    mode_changed = true;
                }
            }
            Key::CtrlF => {
                if toggle_active_maximize(&mut self.applications, &self.mode, self.current_desktop, self.last_size.0, self.last_size.1) {
                    mode_changed = true;
                }
            }
            Key::CtrlN => {
                if focus_relative_window(&mut self.applications, &mut self.mode, self.current_desktop, false) {
                    mode_changed = true;
                }
            }
            Key::CtrlP => {
                if focus_relative_window(&mut self.applications, &mut self.mode, self.current_desktop, true) {
                    mode_changed = true;
                }
            }
            Key::CtrlW => {
                if let Some(idx) = wm::active_window_idx(&self.applications, &self.mode, self.current_desktop) {
                    if self.applications[idx].terminal.is_some() {
                        if let Some(t) = self.applications[idx].terminal.as_mut() {
                            if let Some(mut session) = t.shell_session.take() {
                                session.kill();
                            }
                        }
                    }
                }
                if close_active_window(&mut self.applications, &mut self.mode, self.current_desktop, self.last_size.1, &mut self.tab_scroll) {
                    mode_changed = true;
                }
            }

            Key::CtrlT => {
                let app_idx = spawn_terminal_window(
                    &mut self.applications,
                    &mut self.next_terminal_id,
                    self.current_desktop,
                    self.last_size.0,
                    self.last_size.1,
                    &self.current_path,
                    Vec::new(),
                );
                place_pointer_on_terminal_input(&mut self.pointer, &self.applications, app_idx, self.last_size.0, self.last_size.1);
                self.mode = Mode::TerminalFocus { app_idx };
                mode_changed = true;
            }
            Key::AltR => {
                if enter_active_resize_mode(
                    &self.applications,
                    &mut self.mode,
                    self.current_desktop,
                    &mut self.pointer,
                    self.last_size.0,
                    self.last_size.1,
                ) {
                    mode_changed = true;
                }
            }
            Key::AltV => {
                if let Some(app_idx) = split_active_terminal_window(
                    &mut self.applications,
                    &mut self.mode,
                    &mut self.next_terminal_id,
                    self.current_desktop,
                    wm::SplitDirection::Vertical,
                ) {
                    place_pointer_on_terminal_input(&mut self.pointer, &self.applications, app_idx, self.last_size.0, self.last_size.1);
                    self.mode = Mode::TerminalFocus { app_idx };
                    mode_changed = true;
                }
            }
            Key::AltH => {
                if let Some(app_idx) = split_active_terminal_window(
                    &mut self.applications,
                    &mut self.mode,
                    &mut self.next_terminal_id,
                    self.current_desktop,
                    wm::SplitDirection::Horizontal,
                ) {
                    place_pointer_on_terminal_input(&mut self.pointer, &self.applications, app_idx, self.last_size.0, self.last_size.1);
                    self.mode = Mode::TerminalFocus { app_idx };
                    mode_changed = true;
                }
            }

            Key::CtrlC => {
                let focused_idx = match &self.mode {
                    Mode::TerminalFocus { app_idx } => Some(*app_idx),
                    _ => None,
                };
                if let Some(idx) = focused_idx {
                    if let Some(t) = self.applications[idx].terminal.as_mut() {
                        if let Some(ref mut session) = t.shell_session {
                            // Forward ^C to the live shell session.
                            session.write(&[3]);
                            mode_changed = true;
                        } else {
                            // Fallback: interrupt a running one-shot command.
                            for cmd in t.commands.iter_mut().rev() {
                                if cmd.is_running_external() {
                                    cmd.kill();
                                    t.commands.push(CommandEntry::completed(
                                        "^C", &t.path, vec!["".to_string()],
                                    ));
                                    mode_changed = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            _ => {
                let kind = ModeKind::of(&self.mode);
                let app_idx = self.mode_app_idx();
                mode_changed = match kind {
                    ModeKind::Normal => self.key_normal(key),
                    ModeKind::Typing => self.key_typing(key),
                    ModeKind::Moving => self.key_moving(key, app_idx.expect("Moving carries app_idx")),
                    ModeKind::Resizing => self.key_resizing(key, app_idx.expect("Resizing carries app_idx")),
                    ModeKind::TerminalFocus => self.key_terminal_focus(key, app_idx.expect("TerminalFocus carries app_idx")),
                };
            }
        }

        mode_changed
    }

    fn key_normal(&mut self, key: Key) -> bool {
        let mut mode_changed = false;
        match key {
            Key::AltUp | Key::AltDown | Key::AltLeft | Key::AltRight => {
                if let Some(region) = resolve_snap_region(&key, os::held_arrow_keys()) {
                    if snap_active_window(&mut self.applications, &mut self.mode, self.current_desktop, self.last_size.0, self.last_size.1, region) {
                        mode_changed = true;
                    }
                }
            }
            Key::Char(digit @ '1'..='4') => {
                self.current_desktop = digit.to_digit(10).unwrap_or(1) as usize;
                self.tab_scroll = self.tab_scroll.min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
                if !wm::mode_targets_desktop(&self.mode, &self.applications, self.current_desktop) {
                    self.mode = Mode::Normal;
                }
                mode_changed = true;
            }
            Key::CtrlD => {
                if toggle_start_menu(&mut self.applications, self.current_desktop, self.last_size.1, &mut self.tab_scroll) {
                    mode_changed = true;
                }
            }
            Key::CtrlX => {
                if minimize_active_window(&mut self.applications, &mut self.mode, self.current_desktop, self.last_size.1, &mut self.tab_scroll) {
                    mode_changed = true;
                }
            }
            Key::Up    => self.pointer.move_up(),
            Key::Down  => self.pointer.move_down(self.last_size.1),
            Key::Left  => self.pointer.move_left(),
            Key::Right => self.pointer.move_right(self.last_size.0),

            Key::Home => {
                self.pointer.x = CMD_INPUT_X;
                self.pointer.y = self.last_size.1 - 2;
            }

            Key::Char(':') => {
                self.pointer.x = CMD_INPUT_X;
                self.pointer.y = self.last_size.1 - 2;
                self.mode = Mode::Typing;
                self.panel_scroll = 0;
                mode_changed = true;
            }

            Key::Char(' ') | Key::Enter => {
                let sb_x   = self.last_size.0.saturating_sub(1);
                let sb_top = 1u16;
                let sb_bot = self.last_size.1.saturating_sub(4);
                let tab_x  = self.last_size.0.saturating_sub(3);

                if let Some(d) = desktop_at(self.pointer.x, self.pointer.y, self.last_size.0, self.last_size.1) {
                    self.current_desktop = d;
                    self.tab_scroll = self.tab_scroll.min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
                    if !wm::mode_targets_desktop(&self.mode, &self.applications, self.current_desktop) {
                        self.mode = Mode::Normal;
                    }
                    mode_changed = true;
                } else if self.pointer.y == self.last_size.1 - 2
                    && self.pointer.x >= CMD_INPUT_X.saturating_sub(TERMINAL_INPUT_PREFIX.len() as u16)
                {
                    self.mode = Mode::Typing;
                    self.panel_scroll = 0;
                    mode_changed = true;
                } else {
                    let start_end = STATUS_START_X + STATUS_START.len() as u16;
                    if self.pointer.y == self.last_size.1 - 2
                        && self.pointer.x >= STATUS_START_X
                        && self.pointer.x < start_end
                    {
                        toggle_start_menu(&mut self.applications, self.current_desktop, self.last_size.1, &mut self.tab_scroll);
                        mode_changed = true;
                    } else if self.pointer.x == sb_x {
                        self.last_space_time = None;
                        let mid = (sb_top + sb_bot) / 2;
                        if self.pointer.y <= mid {
                            self.tab_scroll = self.tab_scroll.saturating_sub(1);
                        } else {
                            self.tab_scroll = (self.tab_scroll + 1)
                                .min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
                        }
                        mode_changed = true;
                    } else if self.pointer.x >= tab_x {
                        self.last_space_time = None;
                        let on_tab = tab_layout(&self.applications, self.current_desktop, self.last_size.1, self.tab_scroll)
                            .into_iter()
                            .find(|&(_, ty, th)| self.pointer.y >= ty && self.pointer.y < ty + th)
                            .map(|(idx, _, _)| idx);

                        if let Some(app_idx) = on_tab {
                            self.applications[app_idx].restore();
                            let restored_idx = bring_window_to_front(&mut self.applications, app_idx);
                            self.tab_scroll = self.tab_scroll
                                .min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
                            if self.applications[restored_idx].terminal.is_some() {
                                place_pointer_on_terminal_input(&mut self.pointer, &self.applications, restored_idx, self.last_size.0, self.last_size.1);
                                self.mode = Mode::TerminalFocus { app_idx: restored_idx };
                            }
                            mode_changed = true;
                        }
                    } else if let Some(top_idx) =
                        topmost_window_at(&self.applications, self.current_desktop, self.pointer.x, self.pointer.y)
                    {
                        let mut skip = false;
                        if let Some(menu_idx) = self.applications.iter().position(|a| a.on_desktop(self.current_desktop) && a.is_menu) {
                            if top_idx != menu_idx {
                                self.applications.remove(menu_idx);
                                self.tab_scroll = self.tab_scroll
                                    .min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
                                mode_changed = true;
                                skip = true;
                            }
                        }
                        if !skip {
                            let scroll_handled = if let Some(app) = self.applications.get_mut(top_idx) {
                                let handled = if let Some(win) = app.window_mut() {
                                    win.interact(self.pointer.x, self.pointer.y)
                                } else {
                                    false
                                };
                                handled || wm::interact_terminal_horizontal_scroll(app, self.pointer.x, self.pointer.y)
                                    || wm::interact_terminal_vertical_scroll(app, self.pointer.x, self.pointer.y)
                            } else {
                                false
                            };
                            if scroll_handled {
                                mode_changed = true;
                            }

                            let is_terminal_input = {
                                let app = &self.applications[top_idx];
                                app.terminal.is_some() && app.window().map_or(false, |win| {
                                    let has_hscroll = win.content_w as usize > win.width.saturating_sub(2) as usize;
                                    win.height >= 5
                                        && self.pointer.y == win.position_y + win.height.saturating_sub(if has_hscroll { 3 } else { 2 })
                                        && self.pointer.x > win.position_x
                                        && self.pointer.x < win.position_x + win.width - 1
                                })
                            };
                            if is_terminal_input && !scroll_handled {
                                if top_idx != self.applications.len() - 1 {
                                    let app = self.applications.remove(top_idx);
                                    self.applications.push(app);
                                }
                                place_pointer_on_terminal_input(&mut self.pointer, &self.applications, self.applications.len() - 1, self.last_size.0, self.last_size.1);
                                self.mode = Mode::TerminalFocus { app_idx: self.applications.len() - 1 };
                                mode_changed = true;
                            }

                            if !scroll_handled && !is_terminal_input {
                                let (is_minimize, is_close, is_resize, is_title, offset_x,
                                     win_minimizable, win_closable, win_draggable, win_resizable) = {
                                    let win = self.applications[top_idx].window().unwrap();
                                    let lx = win.position_x;
                                    let rx = win.position_x + win.width - 1;
                                    let ty = win.position_y;
                                    let by = win.position_y + win.height - 1;
                                    (
                                        self.pointer.x == lx && self.pointer.y == ty,
                                        self.pointer.x == rx && self.pointer.y == ty,
                                        self.pointer.x == rx && self.pointer.y == by,
                                        self.pointer.y == ty && self.pointer.x > lx && self.pointer.x < rx,
                                        self.pointer.x.saturating_sub(lx),
                                        win.minimizable,
                                        win.closable,
                                        win.draggable,
                                        win.resizable,
                                    )
                                };
                                let maximized = self.applications[top_idx].is_maximized();

                                if is_minimize && win_minimizable {
                                    self.applications[top_idx].minimize();
                                    mode_changed = true;
                                } else if is_close && win_closable {
                                    if let Some(t) = self.applications[top_idx].terminal.as_mut() {
                                        if let Some(mut session) = t.shell_session.take() {
                                            session.kill();
                                        }
                                    }
                                    self.applications.remove(top_idx);
                                    self.tab_scroll = self.tab_scroll
                                        .min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
                                    mode_changed = true;
                                } else if is_resize && !maximized && win_resizable {
                                    self.mode = Mode::Resizing { app_idx: top_idx, edit: None };
                                    mode_changed = true;
                                } else if is_title && win_draggable {
                                    let now = Clock::now();
                                    let is_double = self.last_space_time
                                        .as_ref()
                                        .map(|t| t.elapsed() < Duration::from_millis(300))
                                        .unwrap_or(false);
                                    self.last_space_time = if is_double { None } else { Some(now) };

                                    if is_double {
                                        if maximized {
                                            self.applications[top_idx].restore_maximize();
                                        } else {
                                            self.applications[top_idx].maximize(self.last_size.0, self.last_size.1);
                                        }
                                        mode_changed = true;
                                    } else if !maximized {
                                        let final_idx = if top_idx != self.applications.len() - 1 {
                                            let app = self.applications.remove(top_idx);
                                            self.applications.push(app);
                                            self.applications.len() - 1
                                        } else {
                                            top_idx
                                        };
                                        self.mode = Mode::Moving { app_idx: final_idx, offset_x };
                                        mode_changed = true;
                                    }
                                } else {
                                    self.last_space_time = None;
                                    if top_idx != self.applications.len() - 1 {
                                        let app = self.applications.remove(top_idx);
                                        self.applications.push(app);
                                        mode_changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        mode_changed
    }

    fn key_typing(&mut self, key: Key) -> bool {
        let mut mode_changed = false;
        match key {
            Key::Escape | Key::End => {
                self.mode = Mode::Normal;
                mode_changed = true;
            }
            Key::CtrlEnter => {
                let cmds = std::mem::take(&mut self.commands);
                self.cmd_input.clear();
                self.cmd_cursor = 0;
                self.panel_scroll = 0;
                let app_idx = spawn_terminal_window(
                    &mut self.applications,
                    &mut self.next_terminal_id,
                    self.current_desktop,
                    self.last_size.0,
                    self.last_size.1,
                    &self.current_path,
                    cmds,
                );
                place_pointer_on_terminal_input(&mut self.pointer, &self.applications, app_idx, self.last_size.0, self.last_size.1);
                self.mode = Mode::TerminalFocus { app_idx };
                mode_changed = true;
            }
            Key::PageUp => {
                self.panel_scroll = self.panel_scroll.saturating_add(1);
                mode_changed = true;
            }
            Key::PageDown => {
                self.panel_scroll = self.panel_scroll.saturating_sub(1);
                mode_changed = true;
            }
            Key::Up => {
                if input::history_up(&self.commands, &mut self.cmd_input, &mut self.history_index, &mut self.history_draft) {
                    self.cmd_cursor = input::input_char_len(&self.cmd_input);
                    mode_changed = true;
                }
            }
            Key::Down => {
                if input::history_down(&self.commands, &mut self.cmd_input, &mut self.history_index, &mut self.history_draft) {
                    self.cmd_cursor = input::input_char_len(&self.cmd_input);
                    mode_changed = true;
                }
            }
            Key::Left => {
                if input::move_input_cursor_left(&mut self.cmd_cursor) {
                    mode_changed = true;
                }
            }
            Key::Right => {
                if input::move_input_cursor_right(&self.cmd_input, &mut self.cmd_cursor) {
                    mode_changed = true;
                }
            }
            Key::Tab => {
                input::reset_history_navigation(&mut self.history_index, &mut self.history_draft);
                if input::autocomplete_input(&mut self.cmd_input, &mut self.cmd_cursor, &self.current_path) {
                    mode_changed = true;
                }
            }
            Key::Enter => {
                let trimmed = self.cmd_input.trim().to_string();
                if !trimmed.is_empty() {
                    let (command, flagged) = split_interactive_flag(&trimmed);
                    let program_hint = command.split_whitespace().next().unwrap_or("").to_string();
                    let interactive = flagged || (!command.is_empty() && is_interactive_app(&program_hint));

                    // `#i app` (or an interactive-list app) opens an interactive terminal
                    // running the program; a bare `#i` opens the default shell interactively.
                    let interactive_program: Option<String> = if flagged && command.is_empty() {
                        Some(crate::app::terminal::default_shell())
                    } else if interactive && !command.is_empty() {
                        Some(command.clone())
                    } else {
                        None
                    };

                    if let Some(program) = interactive_program {
                        let app_idx = spawn_interactive_terminal(
                            &mut self.applications,
                            &mut self.next_terminal_id,
                            self.current_desktop,
                            self.last_size.0,
                            self.last_size.1,
                            &self.current_path,
                            &program,
                        );
                        place_pointer_on_terminal_input(
                            &mut self.pointer,
                            &self.applications,
                            app_idx,
                            self.last_size.0,
                            self.last_size.1,
                        );
                        self.mode = Mode::TerminalFocus { app_idx };
                    } else {
                        push_shell_command(&mut self.commands, &mut self.current_path, &trimmed);
                    }
                    self.history.append(&trimmed);
                    self.cmd_input.clear();
                    self.cmd_cursor = 0;
                    input::reset_history_navigation(&mut self.history_index, &mut self.history_draft);
                    self.panel_scroll = 0;
                }
                mode_changed = true;
            }
            Key::Delete => {
                input::reset_history_navigation(&mut self.history_index, &mut self.history_draft);
                if input::delete_input_char(&mut self.cmd_input, &mut self.cmd_cursor) {
                    mode_changed = true;
                }
            }
            Key::Backspace => {
                input::reset_history_navigation(&mut self.history_index, &mut self.history_draft);
                if input::backspace_input_char(&mut self.cmd_input, &mut self.cmd_cursor) {
                    mode_changed = true;
                }
            }
            Key::Char(c) => {
                input::reset_history_navigation(&mut self.history_index, &mut self.history_draft);
                input::insert_input_char(&mut self.cmd_input, &mut self.cmd_cursor, c);
                mode_changed = true;
            }
            _ => {}
        }
        mode_changed
    }

    fn key_moving(&mut self, key: Key, app_idx: usize) -> bool {
        let mut mode_changed = false;
        match key {
            Key::Up    => self.pointer.move_up(),
            Key::Down  => self.pointer.move_down(self.last_size.1),
            Key::Left  => self.pointer.move_left(),
            Key::Right => self.pointer.move_right(self.last_size.0),
            Key::Char(' ') | Key::Enter => {
                let is_double = self.last_space_time
                    .as_ref()
                    .map(|t| t.elapsed() < Duration::from_millis(300))
                    .unwrap_or(false);
                self.last_space_time = None;
                self.mode = Mode::Normal;
                if is_double {
                    self.applications[app_idx].maximize(self.last_size.0, self.last_size.1);
                }
                mode_changed = true;
            }
            _ => {}
        }
        mode_changed
    }

    fn key_resizing(&mut self, key: Key, app_idx: usize) -> bool {
        // Temporarily take the numeric edit state out of the mode so the key
        // handling below can borrow self freely; it is restored at the end
        // unless the mode changed away from Resizing.
        let mut edit = match &mut self.mode {
            Mode::Resizing { edit, .. } => edit.take(),
            _ => return false,
        };

        let mut mode_changed = false;

        match key {
            Key::Escape => {
                if edit.is_some() {
                    edit = None;
                } else {
                    self.mode = Mode::Normal;
                }
                mode_changed = true;
            }
            Key::Char('x') | Key::Char('h') => {
                edit = Some(ResizeEditState { axis: wm::ResizeAxis::Width, op: None, value: String::new() });
                mode_changed = true;
            }
            Key::Char('y') | Key::Char('v') => {
                edit = Some(ResizeEditState { axis: wm::ResizeAxis::Height, op: None, value: String::new() });
                mode_changed = true;
            }
            _ if edit.is_some() => {
                let mut clear_edit = false;
                let mut changed_pointer = false;

                if let Some(state) = edit.as_mut() {
                    match key {
                        Key::Char(' ') => {}
                        Key::Char('+') if state.op.is_none() => {
                            state.op = Some(wm::ResizeOp::Add);
                            mode_changed = true;
                        }
                        Key::Char('-') if state.op.is_none() => {
                            state.op = Some(wm::ResizeOp::Sub);
                            mode_changed = true;
                        }
                        Key::Char('=') if state.op.is_none() => {
                            state.op = Some(wm::ResizeOp::Set);
                            mode_changed = true;
                        }
                        Key::Char(c) if state.op.is_some() && c.is_ascii_digit() => {
                            state.value.push(c);
                            mode_changed = true;
                        }
                        Key::Backspace if state.op.is_some() && !state.value.is_empty() => {
                            state.value.pop();
                            mode_changed = true;
                        }
                        Key::Enter => {
                            if let Some(win) = self.applications[app_idx].window() {
                                if !state.value.is_empty() {
                                    changed_pointer = apply_resize_edit(win, &mut self.pointer, self.last_size.0, self.last_size.1, state);
                                }
                            }
                            clear_edit = true;
                            mode_changed = true;
                        }
                        _ => {
                            clear_edit = true;
                            mode_changed = true;
                        }
                    }
                }

                if clear_edit {
                    edit = None;
                }
                if changed_pointer {
                    self.pointer.clamp_to_bounds(self.last_size.0, self.last_size.1);
                }
            }
            Key::Up    => self.pointer.move_up(),
            Key::Down  => self.pointer.move_down(self.last_size.1),
            Key::Left  => self.pointer.move_left(),
            Key::Right => self.pointer.move_right(self.last_size.0),
            Key::Char(' ') | Key::Enter => {
                if let Some(win) = self.applications[app_idx].window_mut() {
                    let (width, height) = wm::resize_preview_size(win, &self.pointer);
                    win.width = width;
                    win.height = height;
                }
                self.mode = Mode::Normal;
                mode_changed = true;
            }
            _ => {}
        }

        if let Mode::Resizing { edit: slot, .. } = &mut self.mode {
            *slot = edit;
        }

        mode_changed
    }

    fn key_terminal_focus(&mut self, key: Key, app_idx: usize) -> bool {
        let mut mode_changed = false;
        match key {
            Key::Escape | Key::End => {
                self.mode = Mode::Normal;
                mode_changed = true;
            }
            Key::PageUp => {
                if let Some(t) = self.applications[app_idx].terminal.as_mut() {
                    t.panel_scroll = t.panel_scroll.saturating_add(1);
                    mode_changed = true;
                }
            }
            Key::PageDown => {
                if let Some(t) = self.applications[app_idx].terminal.as_mut() {
                    t.panel_scroll = t.panel_scroll.saturating_sub(1);
                    mode_changed = true;
                }
            }
            _ => {
                if let Some(t) = self.applications[app_idx].terminal.as_mut() {
                    if t.shell_session.is_some() {
                        // Line mode: local echo while typing; the full command
                        // is sent to the shell on Enter.
                        match key {
                            Key::Char(c) => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                input::insert_input_char(&mut t.cmd_input, &mut t.input_cursor, c);
                                mode_changed = true;
                            }
                            Key::Backspace => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                if input::backspace_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Delete => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                if input::delete_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Left => {
                                if input::move_input_cursor_left(&mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Right => {
                                if input::move_input_cursor_right(&t.cmd_input, &mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Home => {
                                t.input_cursor = 0;
                                mode_changed = true;
                            }
                            Key::End => {
                                t.input_cursor = input::input_char_len(&t.cmd_input);
                                mode_changed = true;
                            }
                            Key::Up => {
                                if input::history_up(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                    t.input_cursor = input::input_char_len(&t.cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Down => {
                                if input::history_down(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                    t.input_cursor = input::input_char_len(&t.cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Tab => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                if input::autocomplete_input(&mut t.cmd_input, &mut t.input_cursor, &t.path) {
                                    mode_changed = true;
                                }
                            }
                            Key::Enter => {
                                let cmd = t.cmd_input.trim().to_string();
                                if !cmd.is_empty() {
                                    // Local echo + send to the shell.
                                    if t.has_session() {
                                        t.push_shell_line(cmd.clone());
                                        // Record in the local navigation history.
                                        t.commands.push(CommandEntry::completed(&cmd, &t.path, Vec::new()));
                                        const MAX_HISTORY: usize = 200;
                                        if t.commands.len() > MAX_HISTORY {
                                            t.commands.drain(..t.commands.len() - MAX_HISTORY);
                                        }
                                        // Known REPLs: in the pipe fallback (no real PTY) the
                                        // interactive form requires `-i`; rewrite transparently.
                                        // Send with a `\r\n` line ending (Python and many programs
                                        // require `\n`; a bare `\r` does not make them process the
                                        // line). REPL exit commands (exit/quit) only terminate the
                                        // child, not the session — clear REPL mode so the window
                                        // returns to normal.
                                        if t.repl_prompt.is_some() && is_repl_exit(&cmd) {
                                            t.clear_repl();
                                        }
                                        let line = format!("{}\r\n", interactive_command(&cmd));
                                        if let Some(ref mut session) = t.shell_session {
                                            session.write(line.as_bytes());
                                        }
                                    }
                                    t.cmd_input.clear();
                                    t.input_cursor = 0;
                                    input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    t.panel_scroll = 0;
                                }
                                mode_changed = true;
                            }
                            Key::CtrlD => {
                                // EOF/EOF-ish for tools that use Ctrl+D (python 3, shells).
                                t.clear_repl();
                                if let Some(ref mut session) = t.shell_session {
                                    session.write(&[4]);
                                }
                                mode_changed = true;
                            }
                            Key::CtrlZ => {
                                // EOF on Windows (python2 uses Ctrl+Z+Enter; Ctrl+Z also
                                // suspends jobs on unix shells). Forward the raw byte.
                                t.clear_repl();
                                if let Some(ref mut session) = t.shell_session {
                                    session.write(&[26]);
                                }
                                mode_changed = true;
                            }
                            _ => {}
                        }
                    } else {
                        // Fallback without a session: local editing (legacy).
                        match key {
                            Key::Up => {
                                if input::history_up(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                    t.input_cursor = input::input_char_len(&t.cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Down => {
                                if input::history_down(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                    t.input_cursor = input::input_char_len(&t.cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Left => {
                                if input::move_input_cursor_left(&mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Right => {
                                if input::move_input_cursor_right(&t.cmd_input, &mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Tab => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                if input::autocomplete_input(&mut t.cmd_input, &mut t.input_cursor, &t.path) {
                                    mode_changed = true;
                                }
                            }
                            Key::Enter => {
                                let trimmed = t.cmd_input.trim().to_string();
                                if !trimmed.is_empty() {
                                    push_shell_command(&mut t.commands, &mut t.path, &trimmed);
                                    t.cmd_input.clear();
                                    t.input_cursor = 0;
                                    input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    t.panel_scroll = 0;
                                }
                                mode_changed = true;
                            }
                            Key::Delete => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                if input::delete_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Backspace => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                if input::backspace_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Char(c) => {
                                input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                input::insert_input_char(&mut t.cmd_input, &mut t.input_cursor, c);
                                mode_changed = true;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        mode_changed
    }

    /// Interactive passthrough: every key forwards raw to the session except
    /// Esc/End (leave to the desktop) and PageUp/PageDown (Manto scrollback).
    /// Ctrl+Delete quits globally in `step_input` before this runs.
    fn key_interactive(&mut self, app_idx: usize, key: Key) -> bool {
        match key {
            Key::Escape | Key::End => {
                self.mode = Mode::Normal;
                true
            }
            Key::PageUp | Key::PageDown => {
                // Scroll Manto's scrollback view; the app never sees these.
                if let Some(t) = self.applications.get_mut(app_idx).and_then(|a| a.terminal.as_mut()) {
                    match key {
                        Key::PageUp => t.panel_scroll = t.panel_scroll.saturating_add(1),
                        _ => t.panel_scroll = t.panel_scroll.saturating_sub(1),
                    }
                }
                true
            }
            _ => {
                let is_enter = matches!(key, Key::Enter | Key::CtrlEnter);
                let bytes = crate::app::terminal::key_to_bytes(key);
                if let Some(bytes) = bytes {
                    if let Some(t) = self.applications.get_mut(app_idx).and_then(|a| a.terminal.as_mut()) {
                        let is_pty = t.shell_session.as_ref().map(|s| s.is_real_pty()).unwrap_or(false);
                        // Piped fallback has no real echo: mirror typed input
                        // into the emulator so it stays visible.
                        if !is_pty {
                            if let Some(em) = t.emulator.as_mut() {
                                if is_enter {
                                    em.process(b"\r\n");
                                } else {
                                    em.process(&bytes);
                                }
                            }
                        }
                        if let Some(ref mut s) = t.shell_session {
                            let _ = s.write(&bytes);
                        }
                    }
                }
                // Redraw: the app may repaint even when the key maps to nothing.
                true
            }
        }
    }
}

enum ModeKind {
    Normal,
    Typing,
    Moving,
    Resizing,
    TerminalFocus,
}

impl ModeKind {
    fn of(mode: &Mode) -> Self {
        match mode {
            Mode::Normal => ModeKind::Normal,
            Mode::Typing => ModeKind::Typing,
            Mode::Moving { .. } => ModeKind::Moving,
            Mode::Resizing { .. } => ModeKind::Resizing,
            Mode::TerminalFocus { .. } => ModeKind::TerminalFocus,
        }
    }
}
