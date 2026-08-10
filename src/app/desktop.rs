// Desktop session state: every piece of mutable state of the running
// environment, plus the input handling that drives it.

use std::io::Write;
use std::time::Duration;

use super::terminal::{default_shell, is_interactive_app, split_interactive_flag};
use super::Application;
use crate::cmd::{tick_all, CommandEntry};
use crate::config::{Action, Config};
use crate::input::{self, History};
use crate::menu::{self, MenuItem, MenuKind};
use crate::os::{self, Clock, Key, MouseAction, MouseButton, MouseEvent};
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
    /// Text of the last rendered frame (free screen selection source).
    pub screen: crate::ui::screen::ScreenGrid,
    /// Free screen selection box (screen row/col).
    pub sel: Option<crate::ui::screen::BoxSelect>,
    /// Cell where the selection cursor sits (x, y).
    pub sel_pos: Option<(u16, u16)>,
    /// Manto clipboard (in-memory), synced to the OS clipboard on copy.
    pub clipboard: String,
    /// User configuration (theme + remappable shortcuts).
    pub config: Config,
    /// Force a full clear on the next frame (host terminal resized).
    pub full_redraw: bool,
    /// When the mouse was last used; gates pointer visibility in interactive
    /// apps so it does not double with the app's own cursor.
    pub last_mouse: Option<Clock>,
    /// Master switch for pointer events (Ctrl+M toggles). When off, mouse
    /// input is ignored entirely and the pointer is driven only by the
    /// keyboard.
    pub mouse_enabled: bool,
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

        let config = Config::load();

        // Restore the saved desktop layout (window geometry + active desktop).
        let mut applications = Vec::new();
        sync_terminal_window_metrics(&mut applications);
        let mut current_desktop = 1;
        if let Some(session) = crate::session::load() {
            current_desktop = session.current_desktop.clamp(1, crate::ui::DESKTOP_COUNT);
            let mut next_id = 1usize;
            for saved in session.apps {
                let w = saved.w.clamp(crate::ui::window::MIN_W, last_size.0.saturating_sub(2).max(5));
                let h = saved.h.clamp(crate::ui::window::MIN_H, last_size.1.saturating_sub(4).max(3));
                let x = saved.x.min(last_size.0.saturating_sub(w));
                let y = saved.y.min(last_size.1.saturating_sub(h + 3));
                let cwd = if crate::session::path_exists(&saved.path) {
                    saved.path.clone()
                } else {
                    current_path.clone()
                };
                let idx = crate::wm::spawn_terminal_window_at(
                    &mut applications,
                    &mut next_id,
                    saved.desktop.clamp(1, crate::ui::DESKTOP_COUNT),
                    x, y, w, h,
                    &cwd,
                    Vec::new(),
                );
                if let Some(app) = applications.get_mut(idx) {
                    app.title = saved.title;
                }
            }
        }
        sync_terminal_window_metrics(&mut applications);
        let next_terminal_id = applications.len().saturating_add(1).max(1);

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
            current_desktop,
            next_terminal_id,
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
            screen: crate::ui::screen::ScreenGrid::new(last_size.0, last_size.1),
            sel: None,
            sel_pos: None,
            clipboard: String::new(),
            config,
            full_redraw: true,
            last_mouse: None,
            mouse_enabled: true,
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
        let draw_pointer = self.draw_pointer();
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
            &mut self.screen,
            self.sel.as_ref(),
            self.config.theme,
            self.full_redraw,
            draw_pointer,
        );
        self.full_redraw = false;
    }

    /// Whether to draw the Manto pointer this frame. Hidden while typing in
    /// the dock or editing a line-mode terminal (a caret marks the position).
    /// Inside an interactive app it is shown only when the mouse has moved
    /// recently, so it does not double with the app's own cursor.
    fn draw_pointer(&self) -> bool {
        match &self.mode {
            Mode::Typing => false,
            Mode::TerminalFocus { app_idx } => {
                let interactive = self.applications.get(*app_idx)
                    .and_then(|a| a.terminal.as_ref())
                    .map_or(false, |t| t.interactive);
                interactive && self.mouse_recent()
            }
            _ => true,
        }
    }

    /// True when the mouse was used within the last few seconds.
    fn mouse_recent(&self) -> bool {
        const IDLE: Duration = Duration::from_millis(3000);
        self.last_mouse.as_ref().map(|t| t.elapsed() < IDLE).unwrap_or(false)
    }

    /// Extend the free screen selection in `dir`, seeded from the pointer.
    fn extend_screen_selection(&mut self, dir: (i32, i32)) {
        let (w, h) = (self.last_size.0.max(1) as usize, self.last_size.1.max(1) as usize);
        let pos = self.sel_pos.unwrap_or((self.pointer.x, self.pointer.y));
        let origin = match self.sel {
            Some(s) => s.anchor,
            None => (pos.1 as usize, pos.0 as usize),
        };
        let (mut r, mut c) = (pos.1 as i32, pos.0 as i32);
        r = (r + dir.1).clamp(0, (h - 1) as i32);
        c = (c + dir.0).clamp(0, (w - 1) as i32);
        self.sel_pos = Some((c as u16, r as u16));
        self.sel = Some(crate::ui::screen::BoxSelect { anchor: origin, extent: (r as usize, c as usize) });
    }

    /// Copy the free screen selection (the visible box under selection) to the
    /// OS + internal clipboards. Returns true when copied.
    fn copy_screen_selection(&mut self) -> bool {
        let Some(sel) = self.sel else { return false };
        let (top, bottom, left, right) = sel.bounds();
        let text = self.screen.box_text(left, top, right, bottom);
        self.clipboard = text.clone();
        let _ = crate::os::clipboard_set(&text);
        self.sel = None;
        self.sel_pos = None;
        true
    }

    /// Read clipboard text: OS clipboard first, Manto in-memory fallback.
    fn read_clipboard(&self) -> Option<String> {
        crate::os::clipboard_get().or_else(|| {
            if self.clipboard.is_empty() { None } else { Some(self.clipboard.clone()) }
        })
    }

    /// Read and handle one key event. Returns true when a redraw is needed.
    pub fn step_input(&mut self) -> bool {
        let key = os::read_key();
        // Quit (remappable) both persists the session and leaves the desktop.
        if self.config.resolve(&key) == Some(Action::Quit) {
            self.save_session();
            self.quit = true;
            return false;
        }
        // Mouse disabled (Ctrl+M): drop pointer events before handling.
        if !self.mouse_enabled && matches!(key, Key::Mouse(_)) {
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
            self.screen.resize(new_size.0, new_size.1);
            self.full_redraw = true;
            if self.sel.is_some() {
                self.sel = None; // geometry changed: drop the stale box
            }
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

        // Pointer events drive the desktop and interactive-terminal forwarding.
        if let Key::Mouse(ev) = key {
            return self.handle_mouse(ev);
        }

        // Remappable desktop shortcuts (theme/shortcuts in ~/.manto/config.json).
        if let Some(action) = self.config.resolve(&key) {
            return self.run_action(action);
        }

        // Start menu open on this desktop: Up/Down navigate the entries,
        // Enter launches the selected one, Esc/Ctrl+D close. Other keys fall
        // through to the normal handler.
        if matches!(self.mode, Mode::Normal)
            && let Some(menu_idx) = self.start_menu_idx()
            && let Some(handled) = self.key_menu(menu_idx, &key)
        {
            return handled;
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
            Key::CtrlC => {
                // Free screen selection copies with Ctrl+C.
                if self.copy_screen_selection() {
                    mode_changed = true;
                    return mode_changed;
                }
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
            Key::Escape if self.sel.is_some() => {
                // Clear the free selection without leaving normal mode.
                self.sel = None;
                self.sel_pos = None;
                mode_changed = true;
            }
            Key::Enter if self.sel.is_some() => {
                // Copy the selected screen box.
                mode_changed = self.copy_screen_selection();
            }
            Key::ShiftUp | Key::ShiftDown | Key::ShiftLeft | Key::ShiftRight => {
                // Free screen selection: anchor at the pointer, follow the arrows.
                self.extend_screen_selection(arrow_dir(key));
                mode_changed = true;
            }
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
                if self.space_action() {
                    mode_changed = true;
                }
            }
            _ => {}
        }
        mode_changed
    }

    /// Click/focus action at the pointer position: switch desktops, open the
    /// typing bar, toggle the start menu, scroll tabs, restore a tab, or
    /// interact with the topmost window (scroll, focus, enter terminal input,
    /// drag / resize / minimize / close). Shared by Space/Enter and the mouse.
    fn space_action(&mut self) -> bool {
        let mut mode_changed = false;
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
                toggle_start_menu(&mut self.applications, self.current_desktop, self.last_size.1, &mut self.tab_scroll, menu::load());
                self.park_pointer_on_start_menu();
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
                        let final_idx = bring_window_to_front(&mut self.applications, top_idx);
                        // Interactive sessions keep the pointer where it
                        // was placed (it anchors the box selection).
                        let interactive = self.applications[final_idx]
                            .terminal.as_ref().map_or(false, |t| t.interactive);
                        if !interactive {
                            place_pointer_on_terminal_input(&mut self.pointer, &self.applications, final_idx, self.last_size.0, self.last_size.1);
                        }
                        self.mode = Mode::TerminalFocus { app_idx: final_idx };
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
                                let final_idx = bring_window_to_front(&mut self.applications, top_idx);
                                self.mode = Mode::Moving { app_idx: final_idx, offset_x };
                                mode_changed = true;
                            }
                        } else {
                            self.last_space_time = None;
                            if top_idx != self.applications.len() - 1 {
                                bring_window_to_front(&mut self.applications, top_idx);
                                mode_changed = true;
                            }
                        }
                    }
                }
            }
        }
        mode_changed
    }

    /// Dispatch a remappable desktop shortcut (from ~/.manto/config.json).
    fn run_action(&mut self, action: Action) -> bool {
        match action {
            Action::NewTerminal => {
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
                true
            }
            Action::CloseWindow => {
                if let Some(idx) = wm::active_window_idx(&self.applications, &self.mode, self.current_desktop) {
                    if self.applications[idx].terminal.is_some() {
                        if let Some(t) = self.applications[idx].terminal.as_mut() {
                            if let Some(mut session) = t.shell_session.take() {
                                session.kill();
                            }
                        }
                    }
                }
                close_active_window(&mut self.applications, &mut self.mode, self.current_desktop, self.last_size.1, &mut self.tab_scroll)
            }
            Action::ToggleMaximize => toggle_active_maximize(
                &mut self.applications,
                &self.mode,
                self.current_desktop,
                self.last_size.0,
                self.last_size.1,
            ),
            Action::StartMenu => {
                if toggle_start_menu(&mut self.applications, self.current_desktop, self.last_size.1, &mut self.tab_scroll, menu::load()) {
                    self.park_pointer_on_start_menu();
                    true
                } else {
                    false
                }
            }
            Action::SplitVertical => {
                if let Some(app_idx) = split_active_terminal_window(
                    &mut self.applications,
                    &mut self.mode,
                    &mut self.next_terminal_id,
                    self.current_desktop,
                    wm::SplitDirection::Vertical,
                ) {
                    place_pointer_on_terminal_input(&mut self.pointer, &self.applications, app_idx, self.last_size.0, self.last_size.1);
                    self.mode = Mode::TerminalFocus { app_idx };
                    true
                } else {
                    false
                }
            }
            Action::SplitHorizontal => {
                if let Some(app_idx) = split_active_terminal_window(
                    &mut self.applications,
                    &mut self.mode,
                    &mut self.next_terminal_id,
                    self.current_desktop,
                    wm::SplitDirection::Horizontal,
                ) {
                    place_pointer_on_terminal_input(&mut self.pointer, &self.applications, app_idx, self.last_size.0, self.last_size.1);
                    self.mode = Mode::TerminalFocus { app_idx };
                    true
                } else {
                    false
                }
            }
            Action::Minimize => minimize_active_window(
                &mut self.applications,
                &mut self.mode,
                self.current_desktop,
                self.last_size.1,
                &mut self.tab_scroll,
            ),
            Action::FocusNext => focus_relative_window(&mut self.applications, &mut self.mode, self.current_desktop, false),
            Action::FocusPrev => focus_relative_window(&mut self.applications, &mut self.mode, self.current_desktop, true),
            Action::ResizeActive => enter_active_resize_mode(
                &self.applications,
                &mut self.mode,
                self.current_desktop,
                &mut self.pointer,
                self.last_size.0,
                self.last_size.1,
            ),
            Action::ToggleMouse => {
                self.mouse_enabled = !self.mouse_enabled;
                true
            }
            Action::Quit => {
                self.save_session();
                self.quit = true;
                false
            }
        }
    }

    /// Handle a pointer event in desktop (non-interactive) context.
    fn handle_mouse(&mut self, ev: MouseEvent) -> bool {
        self.last_mouse = Some(Clock::now());
        // Translate 1-based terminal coordinates to Manto screen coordinates.
        let sx = ev.x.saturating_sub(1);
        let sy = ev.y.saturating_sub(1);
        let moved = self.pointer.x != sx || self.pointer.y != sy;
        self.pointer.x = sx;
        self.pointer.y = sy;
        self.pointer.clamp_to_bounds(self.last_size.0, self.last_size.1);

        match ev.kind {
            MouseAction::Move | MouseAction::Drag => {
                // Hovering over a start-menu entry highlights it.
                if let Some(menu_idx) = self.start_menu_idx()
                    && let Some(sel) = self.menu_item_under_pointer(menu_idx)
                {
                    self.select_menu_item(menu_idx, sel);
                    return true;
                }
                moved
            }
            MouseAction::Release => {
                let was_dragging = matches!(self.mode, Mode::Moving { .. } | Mode::Resizing { .. });
                if was_dragging {
                    if let Mode::Resizing { app_idx, .. } = &self.mode {
                        let app_idx = *app_idx;
                        if let Some(win) = self.applications.get_mut(app_idx).and_then(|a| a.window_mut()) {
                            let (width, height) = wm::resize_preview_size(win, &self.pointer);
                            win.width = width;
                            win.height = height;
                        }
                    }
                    self.mode = Mode::Normal;
                    true
                } else {
                    moved
                }
            }
            MouseAction::Press => match ev.button {
                MouseButton::WheelUp => self.mouse_scroll(true),
                MouseButton::WheelDown => self.mouse_scroll(false),
                MouseButton::Left => self.mouse_left_press(),
                MouseButton::Right => self.mouse_right_press(),
                MouseButton::Middle => false,
            },
        }
    }

    fn mouse_left_press(&mut self) -> bool {
        // Clicking a start-menu entry selects and launches it.
        if let Some(menu_idx) = self.start_menu_idx()
            && let Some(sel) = self.menu_item_under_pointer(menu_idx)
        {
            self.select_menu_item(menu_idx, sel);
            let item = self.applications.get(menu_idx)
                .and_then(|a| a.menu.as_ref())
                .and_then(|s| s.items.get(s.selected))
                .cloned();
            self.close_start_menu(menu_idx);
            if let Some(item) = item {
                return self.launch_menu_item(&item);
            }
            return true;
        }

        // Clicking the body of an interactive terminal dives into it, so the
        // next pointer events reach the app.
        if let Some(top) = topmost_window_at(&self.applications, self.current_desktop, self.pointer.x, self.pointer.y)
            && let Some(win) = self.applications.get(top).and_then(|a| a.window())
            && self.applications[top].terminal.as_ref().map_or(false, |t| t.interactive)
            && self.pointer.x > win.position_x
            && self.pointer.x < win.position_x + win.width - 1
            && self.pointer.y > win.position_y
            && self.pointer.y < win.position_y + win.height - 1
        {
            let idx = bring_window_to_front(&mut self.applications, top);
            self.mode = Mode::TerminalFocus { app_idx: idx };
            return true;
        }

        self.space_action()
    }

    fn mouse_right_press(&mut self) -> bool {
        // Right-click focuses (raises) the window under the pointer.
        if let Some(top_idx) = topmost_window_at(&self.applications, self.current_desktop, self.pointer.x, self.pointer.y) {
            bring_window_to_front(&mut self.applications, top_idx);
            self.tab_scroll = self.tab_scroll.min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
            self.mode = Mode::Normal;
            true
        } else {
            false
        }
    }

    /// Wheel scrolling: the minimized-window rail (right edge), the terminal
    /// under the pointer, or the dock command panel.
    fn mouse_scroll(&mut self, up: bool) -> bool {
        let sb_x = self.last_size.0.saturating_sub(1);
        let sb_top = 1u16;
        let sb_bot = self.last_size.1.saturating_sub(4);

        // Tab / minimization rail on the rightmost column.
        if self.pointer.x == sb_x && self.pointer.y >= sb_top && self.pointer.y <= sb_bot {
            let track = (sb_bot - sb_top + 1) as usize;
            let total = self.applications.iter()
                .filter(|a| a.on_desktop(self.current_desktop) && a.is_minimized())
                .count();
            let tab_h = if (total as u16) * 8 <= (sb_bot - sb_top + 1) { 8 } else { 6 };
            let visible = (track / (tab_h as usize).max(1)).max(1);
            if total > visible {
                self.tab_scroll = if up {
                    self.tab_scroll.saturating_sub(1)
                } else {
                    (self.tab_scroll + 1).min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1))
                };
                return true;
            }
        }

        // Terminal under the pointer: scroll its scrollback.
        if let Some(top) = topmost_window_at(&self.applications, self.current_desktop, self.pointer.x, self.pointer.y)
            && let Some(t) = self.applications.get_mut(top).and_then(|a| a.terminal.as_mut())
        {
            if up {
                t.panel_scroll = t.panel_scroll.saturating_add(1);
            } else {
                t.panel_scroll = t.panel_scroll.saturating_sub(1);
            }
            return true;
        }

        // Dock command panel.
        if !self.commands.is_empty() {
            if up {
                self.panel_scroll = self.panel_scroll.saturating_add(1);
            } else {
                self.panel_scroll = self.panel_scroll.saturating_sub(1);
            }
            return true;
        }
        false
    }

    /// Forward a pointer event to a focused interactive session (SGR), so
    /// mouse-aware apps (vim, mc, ...) track the pointer as in a terminal.
    fn forward_mouse(&mut self, app_idx: usize, ev: MouseEvent) -> bool {
        if let Some(bytes) = crate::app::terminal::mouse_to_bytes(ev) {
            if let Some(t) = self.applications.get_mut(app_idx).and_then(|a| a.terminal.as_mut()) {
                let is_pty = t.shell_session.as_ref().map(|s| s.is_real_pty()).unwrap_or(false);
                if is_pty {
                    if let Some(ref mut s) = t.shell_session {
                        let _ = s.write(&bytes);
                    }
                }
            }
        }
        true
    }

    /// Persist the current layout (terminal geometry + active desktop).
    fn save_session(&self) {
        let mut session = crate::session::Session {
            current_desktop: self.current_desktop,
            apps: Vec::new(),
        };
        for app in &self.applications {
            if app.is_menu { continue; }
            let Some(terminal) = app.terminal.as_ref() else { continue };
            let Some(win) = app.window() else { continue };
            session.apps.push(crate::session::SavedApp {
                title: app.title.clone(),
                path: terminal.path.clone(),
                desktop: app.desktop,
                x: win.position_x,
                y: win.position_y,
                w: win.width,
                h: win.height,
            });
        }
        crate::session::save(&session);
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
            Key::Enter => {
                mode_changed |= self.run_terminal_line(app_idx);
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
                if let Some(t) = self.applications[app_idx].terminal.as_mut() {
                    match key {
                        Key::PageUp => t.panel_scroll = t.panel_scroll.saturating_add(1),
                        _ => t.panel_scroll = t.panel_scroll.saturating_sub(1),
                    }
                }
                true
            }
            Key::Enter | Key::CtrlEnter => self.forward_interactive(app_idx, key, true),
            Key::CtrlC => self.forward_interactive(app_idx, key, false),
            Key::CtrlV => {
                // Paste: OS clipboard first, Manto in-memory fallback.
                let text = self.read_clipboard();
                if let Some(text) = text {
                    if !text.is_empty() {
                        if let Some(t) = self.applications.get_mut(app_idx).and_then(|a| a.terminal.as_mut()) {
                            let is_pty = t.shell_session.as_ref().map(|s| s.is_real_pty()).unwrap_or(false);
                            if let Some(em) = t.emulator.as_mut() {
                                if !is_pty {
                                    em.process(text.as_bytes());
                                }
                            }
                            if is_pty {
                                if let Some(ref mut s) = t.shell_session {
                                    let _ = s.write(text.as_bytes());
                                }
                            } else {
                                // Piped sessions: buffer until Enter so the
                                // pasted text joins the edited line.
                                t.pipe_feed(text.as_bytes());
                            }
                        }
                    }
                }
                true
            }
            Key::Mouse(ev) => {
                // A press outside the terminal window returns to the desktop.
                let inside = self.applications.get(app_idx).and_then(|a| a.window()).map_or(false, |win| {
                    let x = ev.x.saturating_sub(1);
                    let y = ev.y.saturating_sub(1);
                    x >= win.position_x
                        && x < win.position_x + win.width
                        && y >= win.position_y
                        && y < win.position_y + win.height
                });
                if !inside {
                    self.mode = Mode::Normal;
                    return self.handle_mouse(ev);
                }
                self.forward_mouse(app_idx, ev)
            }
            _ => self.forward_interactive(app_idx, key, false),
        }
    }

    /// Forward a key raw to the session, with Manto-side local echo when the
    /// backend has no real PTY.
    fn forward_interactive(&mut self, app_idx: usize, key: Key, enter: bool) -> bool {
        let _ = enter;
        let is_pty = self.applications.get(app_idx)
            .and_then(|a| a.terminal.as_ref())
            .and_then(|t| t.shell_session.as_ref())
            .map(|s| s.is_real_pty())
            .unwrap_or(false);

        // Real PTY: the child/console handles echo, editing and history.
        if is_pty {
            if let Some(bytes) = crate::app::terminal::key_to_bytes(key) {
                if let Some(t) = self.applications.get_mut(app_idx).and_then(|a| a.terminal.as_mut()) {
                    if let Some(ref mut s) = t.shell_session {
                        let _ = s.write(&bytes);
                    }
                }
            }
            return true;
        }

        // Piped fallback: no console to echo/edit/keep history in, so Manto
        // mirrors the keystrokes, edits the partial line locally and recalls a
        // local history with Up/Down. A real console would otherwise leak
        // control bytes into the child or move the emulator's cursor.
        if let Some(t) = self.applications.get_mut(app_idx).and_then(|a| a.terminal.as_mut()) {
            match key {
                Key::Up => {
                    t.pipe_recall(true);
                }
                Key::Down => {
                    t.pipe_recall(false);
                }
                Key::Enter | Key::CtrlEnter => {
                    t.mirror_input(b"\r\n", false);
                    t.pipe_flush();
                }
                Key::Backspace => {
                    t.mirror_input(&crate::app::terminal::key_to_bytes(key).unwrap_or_default(), true);
                    t.pipe_backspace();
                    t.reset_pipe_history();
                }
                Key::CtrlC => {
                    t.mirror_input(b"\r\n", false);
                    t.pipe_cancel();
                }
                // Navigation/function keys have no meaning on a pipe (no
                // terminal to interpret them): ignore, don't mirror or send.
                key if crate::app::terminal::is_terminal_navigation(key) => (),
                _ => {
                    if let Some(bytes) = crate::app::terminal::key_to_bytes(key) {
                        t.reset_pipe_history();
                        t.mirror_input(&bytes, false);
                        t.pipe_feed(&bytes);
                    }
                }
            }
        }
        true
    }

    /// Send the `.>` line: to the session (classic line mode) or, without a
    /// session, through the command runner. Resets the input bar.
    fn run_terminal_line(&mut self, app_idx: usize) -> bool {
        let Some(t) = self.applications[app_idx].terminal.as_mut() else {
            return false;
        };
        let is_session = t.shell_session.is_some();
        let cmd = t.cmd_input.trim().to_string();
        if !cmd.is_empty() {
            if is_session {
                t.run_line(&cmd);
            } else {
                push_shell_command(&mut t.commands, &mut t.path, &cmd);
            }
            t.cmd_input.clear();
            t.input_cursor = 0;
            input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
            t.panel_scroll = 0;
            true
        } else {
            false
        }
    }

    /// Index of the open start menu window on the current desktop.
    fn start_menu_idx(&self) -> Option<usize> {
        self.applications.iter().rposition(|app| {
            app.on_desktop(self.current_desktop) && app.is_menu
        })
    }

    /// Land the pointer on the first menu entry (the "▶" of the initial
    /// selection) when the start menu opens.
    fn park_pointer_on_start_menu(&mut self) {
        if let Some(idx) = self.start_menu_idx()
            && let Some(win) = self.applications[idx].window()
        {
            self.pointer.x = win.position_x + 1;
            self.pointer.y = win.position_y + 1;
            self.pointer.clamp_to_bounds(self.last_size.0, self.last_size.1);
        }
    }

    /// Keyboard handling while the start menu is open (Normal mode only).
    /// Returns None when the key is not a menu key and should fall through
    /// to the normal handler.
    fn key_menu(&mut self, menu_idx: usize, key: &Key) -> Option<bool> {
        // Esc with a free screen selection active clears it first, exactly
        // like normal mode.
        if matches!(key, Key::Escape) && self.sel.is_some() {
            self.sel = None;
            self.sel_pos = None;
            return Some(true);
        }

        // The pointer always moves with the arrows; while it is over the menu
        // the highlighted entry is simply the one under the pointer.
        match key {
            Key::Up | Key::Down | Key::Left | Key::Right => {
                match key {
                    Key::Up => self.pointer.move_up(),
                    Key::Down => self.pointer.move_down(self.last_size.1),
                    Key::Left => self.pointer.move_left(),
                    _ => self.pointer.move_right(self.last_size.0),
                }
                if let Some(selected) = self.menu_item_under_pointer(menu_idx) {
                    self.select_menu_item(menu_idx, selected);
                }
                Some(true)
            }
            Key::Enter | Key::Char(' ') => {
                // Pointer not over an entry: normal click (menu borders,
                // other windows, the Start button itself).
                self.menu_item_under_pointer(menu_idx)?;
                let item = {
                    let state = self.applications.get(menu_idx)?.menu.as_ref()?;
                    state.items.get(state.selected).cloned()
                };
                let Some(item) = item else {
                    return Some(false);
                };
                self.close_start_menu(menu_idx);
                self.launch_menu_item(&item);
                Some(true)
            }
            Key::Escape | Key::CtrlD => {
                self.close_start_menu(menu_idx);
                Some(true)
            }
            _ => None,
        }
    }

    /// Index of the menu entry under the pointer, when the pointer is over an
    /// entry row of the open start menu.
    fn menu_item_under_pointer(&self, menu_idx: usize) -> Option<usize> {
        let app = self.applications.get(menu_idx)?;
        let win = app.window()?;
        let state = app.menu.as_ref()?;
        let (px, py) = (self.pointer.x, self.pointer.y);
        if px < win.position_x + 1 || px >= win.position_x + win.width - 1 {
            return None;
        }
        if py < win.position_y + 1 || py >= win.position_y + win.height - 1 {
            return None;
        }
        let row = (py - win.position_y - 1) as usize;
        let entry = state.scroll + row;
        (entry < state.items.len()).then_some(entry)
    }

    /// Move the menu selection, keeping it inside the visible rows.
    fn select_menu_item(&mut self, menu_idx: usize, selected: usize) {
        let visible = self.applications.get(menu_idx)
            .and_then(|app| app.window())
            .map_or(1, |win| (win.height as usize).saturating_sub(2).max(1));
        if let Some(state) = self.applications[menu_idx].menu.as_mut() {
            state.selected = selected;
            state.keep_selected_visible(visible);
        }
    }

    /// Close the start menu window (it is re-created on the next toggle).
    fn close_start_menu(&mut self, menu_idx: usize) {
        self.applications.remove(menu_idx);
        self.tab_scroll = self.tab_scroll
            .min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
    }

    /// Launch a manifest entry: interactive app, plain terminal, or a
    /// terminal running one command, on the requested desktop (defaults to
    /// the current one).
    fn launch_menu_item(&mut self, item: &MenuItem) -> bool {
        let cwd = item.resolve_cwd(&self.current_path);
        let desktop = item.desktop.unwrap_or(self.current_desktop);
        let command_line = item.command_line();

        let app_idx = match item.kind {
            MenuKind::App => {
                let program = if command_line.is_empty() {
                    default_shell()
                } else {
                    command_line.clone()
                };
                spawn_interactive_terminal(
                    &mut self.applications,
                    &mut self.next_terminal_id,
                    desktop,
                    self.last_size.0,
                    self.last_size.1,
                    &cwd,
                    &program,
                )
            }
            MenuKind::Terminal | MenuKind::Command => {
                let idx = spawn_terminal_window(
                    &mut self.applications,
                    &mut self.next_terminal_id,
                    desktop,
                    self.last_size.0,
                    self.last_size.1,
                    &cwd,
                    Vec::new(),
                );
                if item.kind == MenuKind::Command && !command_line.is_empty() {
                    if let Some(t) = self.applications[idx].terminal.as_mut() {
                        t.cmd_input = command_line;
                        t.input_cursor = t.cmd_input.chars().count();
                    }
                    self.run_terminal_line(idx);
                }
                idx
            }
        };

        if let Some(app) = self.applications.get_mut(app_idx) {
            app.title = item.label.clone();
        }

        if desktop != self.current_desktop {
            self.current_desktop = desktop;
            self.tab_scroll = self.tab_scroll
                .min(max_tab_scroll(&self.applications, self.current_desktop, self.last_size.1));
        }

        place_pointer_on_terminal_input(
            &mut self.pointer,
            &self.applications,
            app_idx,
            self.last_size.0,
            self.last_size.1,
        );
        self.mode = Mode::TerminalFocus { app_idx };
        true
    }
}

/// Direction vector (dx, dy) for arrow keys.
fn arrow_dir(key: Key) -> (i32, i32) {
    match key {
        Key::Up | Key::ShiftUp => (0, -1),
        Key::Down | Key::ShiftDown => (0, 1),
        Key::Left | Key::ShiftLeft => (-1, 0),
        _ => (1, 0),
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


