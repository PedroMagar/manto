# Manto

Manto is a desktop environment designed to run from a terminal.

### TO-DO
- TUI
    - Mouse
    - Interact
- Window
    - Move
    - Resize
    - Scrolls
- Desktop
    - Minimum size
    - Scroll
- File Explorer
- Dock
    - Alignments
    - Buttons
    - Applications List
- Start Menu
    - Start Window
    - App List
    - Shortcut Manager
- Terminal Integration
    - Command runner
    - PTY / ConPTY backend
    - Shell delegation
    - Screen buffer / ANSI interpreter
- App Builder
- run

## Design

### Portability over dependencies

The project avoids third-party crates (for example `crossterm`) in favor of raw ANSI/VT100 escape sequences. The goal is to run on Linux, macOS, Redox OS, and any OS with minimal porting effort.

### OS isolation layer (`os.rs`)

Everything that touches the host OS lives in `os.rs`:
- `Writer` - output abstraction (stdout today, framebuffer or serial tomorrow)
- `Clock` - time abstraction (Instant today, hardware register tomorrow)
- `Key` - keyboard event type
- Platform modules (`#[cfg(unix)]` / `#[cfg(windows)]`) for raw mode, terminal size, polling, and key reading

To port Manto to a new OS, only `os.rs` needs to be replaced. All other files (`terminal.rs`, `gui.rs`, `window.rs`, `pointer.rs`) are OS-agnostic.

### `ansi.rs` - pure ANSI, no OS dependency

`ansi.rs` emits only ANSI/VT100 bytes via `std::io::Write`. It has no platform conditionals. For a `no_std` port, define an equivalent `Write` trait in `os.rs` and change the import here.

### Application vs Window

`Application` is the logical entity (title, state, content). `Window` is the visual presentation (position, size, layer). They are kept separate because some applications may not have a window at all, such as background services, fullscreen apps, or minimized state.

`DisplayMode` encodes how an application is currently presented:
- `Windowed(Window)` - floating bordered window (implemented)
- `Fullscreen`, `Tab`, `Minimized` - planned

### Layer field

`Window` holds a `layer: u16` field for z-order and future transparency support. Applications that overlap will eventually be composited according to their layer.

### Delta-only preview rendering

During window resize, only the new border extensions are drawn on top of the existing frame. The original borders are not erased. This avoids a full redraw on every cursor move and gives a clear ghost-outline preview of the new size.

## Terminal roadmap

### Goal

Manto should avoid reimplementing a shell.

The preferred direction is:
- Run a real shell or command in the backend
- Forward keyboard input to that backend
- Let the shell handle history, autocomplete, prompt editing, and interactive behavior
- Render the resulting screen state inside the Manto window

This means features such as `Up`, `Tab`, `Ctrl+R`, shell history, and completion should ideally be delegated to a real terminal session rather than hardcoded in the UI layer.

### Why a backend alone is not enough

Even when Manto delegates command execution to a real shell, terminal output is not just plain text. Real shells and terminal applications emit control sequences to:
- Move the cursor
- Clear parts of the line or screen
- Rewrite the prompt in place
- Apply colors and attributes
- Switch between normal and alternate screen modes

Because of that, Manto still needs a minimal terminal-facing layer between the process and the UI.

### Recommended architecture

The simplest long-term architecture is:

1. `terminal_backend`
   - Spawns the shell or command
   - Uses PTY on Unix-like systems and ConPTY on Windows
   - Accepts raw input bytes from Manto
   - Produces output bytes and process events
2. `terminal_emulator`
   - Interprets ANSI / VT sequences
   - Maintains a screen buffer and cursor state
   - Tracks scrollback and resize behavior
3. `terminal_view`
   - Draws the current buffer inside Manto windows
   - Maps pointer, focus, and window size to the emulator state

### Host-agnostic terminal architecture

To keep Manto compatible with a future custom OS, the terminal stack should be split into stable logical layers and replaceable host backends.

The recommended boundary is:

1. `terminal_backend`
   - Host-dependent
   - Owns process creation, IO channels, resize notifications, and lifecycle
   - Uses PTY on Unix-like systems
   - Uses ConPTY on Windows
   - Can later use a native console or pseudo-terminal subsystem in the custom OS
2. `terminal_emulator`
   - Host-independent
   - Consumes bytes and terminal events
   - Interprets ANSI / VT behavior
   - Maintains cursor, attributes, scrollback, alternate screen, and visible cells
3. `terminal_view`
   - Host-independent
   - Converts emulator state into Manto window content
   - Owns clipping, focus, scroll integration, and window sizing rules

The important rule is that PTY / ConPTY must be treated as backend implementations, not as the architecture itself.

### Suggested traits

The backend should be modeled as an interface that can be reimplemented later on a custom OS:

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

This keeps the contract stable:
- the backend is responsible for process and terminal session management
- the emulator is responsible for interpreting terminal semantics
- the UI is responsible for presentation only

### What changes on a future custom OS

If Manto later runs on a custom OS, only the backend implementation should need to change materially.

Expected replacements:
- Unix PTY -> native pseudo-terminal or console session
- Windows ConPTY -> native process / console bridge
- host process spawning -> OS-native task spawning
- host pipes / streams -> OS-native IPC or console buffers

The emulator and UI should remain mostly unchanged if this boundary is respected.

### Suggested phases

#### Phase 1: command runner

Start simple:
- Run full commands
- Capture stdout / stderr
- Display line-based output in the internal terminal window
- No real interactive shell behavior yet

This is enough to validate process lifecycle, output collection, and UI integration.

#### Phase 2: real shell backend

After that:
- Start a shell through PTY / ConPTY
- Forward raw keys directly to the shell
- Read raw output back
- Keep Manto responsible only for presentation and focus management

At this phase, shell history and completion can come from the shell itself.

#### Phase 3: richer terminal behavior

Once the backend is stable:
- Handle resize correctly
- Support full-screen terminal programs such as `top`
- Improve scrollback, colors, alternate screen, and cursor behavior

### Start menu integration

The Start menu can build on top of the same execution model.

A good first step is a declarative manifest instead of arbitrary scripting, for example:
- `label`
- `kind` (`command`, `terminal`, `internal`)
- `command`
- `args`
- `cwd`
- `desktop`

That keeps the menu customizable without coupling it too early to a custom scripting engine.
