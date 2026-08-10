# Architecture

## Design Goals

Manto prioritizes portability, low dependency surface, and clear separation between UI logic and host-specific integration. The long-term goal is to keep the project viable both on mainstream operating systems and on a future custom OS.

## Source Layout

The `src/` tree is organized by layer, keeping a clean dependency direction (`os` < `terminal_backend` < `cmd`/`input` < `wm` < `app` < `ui`, with the crate root tying them together):

```
src/
  main.rs              entry point: terminal setup, main loop, teardown
  os/                  host abstraction (Writer, Clock, Key, MouseEvent + platform impls)
    mod.rs
    unix.rs            #[cfg(unix)] platform
    windows.rs         #[cfg(windows)] platform
  ui/                  presentation: ANSI, window chrome, drawing, frame render
    mod.rs             status bar, desktop frame, tabs, scrollbar, constants
    ansi.rs            ANSI / VT100 sequences
    window.rs          Window: geometry, chrome drawing, scroll interaction
    pointer.rs         Pointer: movement and cursor drawing
    panel.rs           command panel (blocks, priority, clipping) + tests
    terminal_view.rs   terminal / shell content rendering
    render.rs          damage-based frame diff + composition + tests
    screen.rs          frame grid (char + style), selection, StampWriter
  app/                 domain state
    mod.rs             Application, DisplayMode
    terminal.rs        TerminalState + REPL helpers + tests
    desktop.rs         Desktop: session state, input handling, tick, draw
  wm/                  window manager actions (snap, split, focus, resize) + tests
  terminal_emulator/   host-independent ANSI/VT emulator (grid, cursor, SGR, alt screen)
    mod.rs             Terminal, Screen, Cell, attributes, parser + tests
  cmd/                 command entries, one-shot runner, built-ins + tests
  input/               line editing, completion, persistent history + tests
    mod.rs
    history.rs
  terminal_backend/    persistent shell sessions (PTY/ConPTY FFI)
    mod.rs             CommandSession + platform selection
    unix.rs
    windows.rs
  config.rs            user config (~/.manto/config.json): theme + remappable shortcuts
  json.rs              minimal zero-dependency JSON parser (shared)
  session.rs           session persistence (~/.manto/session.json)
```

The rule for porting: most host-specific change lives in `os/` and
`terminal_backend/`. `terminal_emulator/` is purely host-independent and
`ui::terminal_view` maps its grid into windows.

## Portability Over Dependencies

The project avoids third-party terminal crates such as `crossterm` and works directly with ANSI / VT100 output plus a thin host abstraction layer. This keeps the runtime model simple and reduces friction for future ports.

## OS Isolation Layer

Everything that depends on the host OS is concentrated in `os/`:

- `Writer`: output abstraction
- `Clock`: time abstraction
- `Key`: keyboard event abstraction (including `Key::Mouse` for pointer events)
- `MouseEvent`: normalized pointer events (button, action, modifiers, cell coordinates)
- clipboard bridge (Win32 FFI on Windows, `xclip`/`xsel`/`wl-copy` on Unix)
- platform-specific modules for raw mode, terminal size, polling, key and mouse decoding

Mouse input is a host concern with two fetch paths:
- Unix: DEC mouse tracking is enabled on stdout (`?1000`/`?1002`/`?1003`/`?1006`
  SGR modes) and reports are decoded from the input stream (SGR `ESC[<b;x;yM/m`
  plus the legacy X10 `ESC[M` form).
- Windows: `ENABLE_MOUSE_INPUT` stays on and `MOUSE_EVENT_RECORD`s are decoded
  from `ReadConsoleInputW`; the same DEC modes are emitted on stdout so VT hosts
  (VS Code / Windows Terminal via ConPTY) relay pointer events, and physical
  conhost clicks reach the same decoder.

`Desktop.mouse_enabled` (default on, toggled with Ctrl+M, configurable) gates
all pointer handling; when off, pointer events are dropped and the pointer is
driven only by the keyboard.

The intended rule is: when porting Manto to another OS, most of the change should happen in `os/`.

## ANSI Layer

`ui/ansi.rs` emits ANSI / VT100 control sequences through `std::io::Write` and does not depend on platform conditionals. This keeps rendering logic separate from OS handling.

## Application And Window Separation

`Application` represents the logical app state. `Window` represents the visible frame and geometry. This separation allows the same application to be windowed, minimized, maximized, or eventually represented in other forms without mixing presentation and app state.

Current display states are centered around:

- `Windowed(Window)`
- `Minimized(Window)`
- `Maximized { display, saved }`

## Layering

Each `Window` carries a `layer` field that is the real z-order: raising a
window (focus, restore, drag-to-front) bumps its layer above everything else,
and drawing, focus selection and hit-testing (`topmost_window_at`,
`active_window_idx`) order windows by `(layer, vector index)`. The backing
vector acts as a stable tiebreaker within the same layer.

## Damage-Based Rendering

Frames are composed in memory and compared to the previous frame; only changed
rows are rewritten, carrying per-cell styles through minimal SGR transitions
(`ui::screen.rs` keeps a char + style grid via `StampWriter`, which parses the
SGR stream to reconstruct absolute styles). A full `clear` happens only when
the host terminal resizes. Free screen selection and hover highlights are
treated as part of the cell style, so attribute-only changes repaint correctly
and flicker is avoided.

## Resize Preview Model

During interactive resize, Manto draws a preview delta instead of fully erasing and redrawing the original window each time. This keeps the visual feedback lightweight and clear.

## Terminal Direction

Manto should avoid reimplementing a shell.

The preferred direction is:

1. run a real shell or command in a backend
2. forward keyboard input to that backend
3. let the shell handle history, completion, prompt editing, and interactive behavior
4. render the resulting state inside the Manto UI

This means features such as shell history and completion should ideally come from a terminal session backend, not from hardcoded UI logic.

Adopted constraints for this direction:

- PTY (Unix) and ConPTY (Windows) are implemented through hand-written FFI, with no external crates. This matches the portability policy above.
- Unix and Windows backends evolve in parallel behind a single `TerminalBackend` interface.

## Why A Backend Alone Is Not Enough

Terminal output is not just plain text. Real terminal programs emit control sequences to:

- move the cursor
- clear portions of the line or screen
- redraw prompts in place
- apply color and text attributes
- switch between normal and alternate screen modes

Because of that, a serious terminal integration eventually needs more than simple process spawning.

## Recommended Terminal Split

The long-term terminal architecture should be split into three layers:

1. `terminal_backend`
   - host-dependent
   - starts processes or shell sessions
   - handles PTY / ConPTY or equivalent
   - forwards input and collects output/events
2. `terminal_emulator`
   - host-independent
   - interprets ANSI / VT behavior
   - maintains cursor, attributes, visible cells, and scrollback
3. `terminal_view`
   - host-independent
   - maps emulator state into Manto windows and focus/scroll behavior

PTY / ConPTY should be treated as backend implementations, not as the architecture itself.

## Host-Agnostic Future

To keep Manto viable for a future custom OS, the backend must stay replaceable.

Expected future replacements:

- Unix PTY -> native pseudo-terminal or console session
- Windows ConPTY -> native console bridge
- host process spawning -> OS-native task spawning
- pipes / streams -> OS-native IPC or console buffers

If this boundary is respected, the emulator and UI should remain mostly unchanged.

## Backend Interface

The backend contract is the single boundary between host and emulator/UI:

```rust
trait TerminalBackend {
    type Id;

    fn spawn(&mut self, program: &str, args: &[String], cwd: Option<&str>) -> Result<Self::Id, String>;
    fn write(&mut self, id: Self::Id, data: &[u8]) -> Result<(), String>;
    fn resize(&mut self, id: Self::Id, cols: u16, rows: u16) -> Result<(), String>;
    fn kill(&mut self, id: Self::Id) -> Result<(), String>;
    fn poll(&mut self) -> Vec<TerminalEvent<Self::Id>>;
}

enum TerminalEvent<I> {
    Output { id: I, bytes: Vec<u8> },
    Exit { id: I, code: Option<i32> },
}
```

This keeps responsibilities clear:

- backend: process/session lifecycle
- emulator: terminal semantics
- UI: presentation

## Start Menu Manifest

The Start menu is driven by a declarative manifest loaded from
`~/.manto/menu.json` (with `example/menu.json` as the reference format).
Useful fields:

- `label`
- `kind` (`app` / `terminal` / `command`)
- `command`
- `args`
- `cwd`
- `desktop`

To preserve the zero-dependency policy, the manifest is read by a minimal
hand-written JSON parser (`json.rs`) rather than re-enabling `serde`.

## User Config And Session Persistence

User configuration lives in `~/.manto/config.json` (theme 0–2 and remappable
desktop shortcuts — including the mouse toggle). Window geometry, titles,
working directories and the active desktop are persisted to
`~/.manto/session.json` on quit and restored on the next start (shell sessions
themselves are host processes and do not survive, but the layout does). Both
files are optional; a broken or missing file falls back to defaults.
