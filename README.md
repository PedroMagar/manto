# Manto

Manto is a desktop environment design to run from terminal.

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
- App Builder
- run

## Design

### Portability over dependencies

The project avoids third-party crates (e.g. crossterm) in favor of raw ANSI/VT100 escape sequences. The goal is to run on Linux, macOS, Redox OS, and any OS with minimal porting effort.

### OS isolation layer (`os.rs`)

Everything that touches the host OS lives in `os.rs`:
- `Writer` — output abstraction (stdout today, framebuffer/serial tomorrow)
- `Clock` — time abstraction (Instant today, hardware register tomorrow)
- `Key` — keyboard event type
- Platform modules (`#[cfg(unix)]` / `#[cfg(windows)]`) for raw mode, terminal size, polling, and key reading

To port Manto to a new OS, only `os.rs` needs to be replaced. All other files (`terminal.rs`, `gui.rs`, `window.rs`, `pointer.rs`) are OS-agnostic.

### `terminal.rs` — pure ANSI, no OS dependency

`terminal.rs` emits only ANSI/VT100 bytes via `std::io::Write`. It has no platform conditionals. For a `no_std` port, define an equivalent `Write` trait in `os.rs` and change the import here.

### Application vs Window

`Application` is the **logical entity** (title, state, content). `Window` is the **visual presentation** (position, size, layer). They are kept separate because some applications may not have a window at all (e.g. background services, fullscreen apps, minimized state).

`DisplayMode` encodes how an application is currently presented:
- `Windowed(Window)` — floating bordered window (implemented)
- `Fullscreen`, `Tab`, `Minimized` — planned

### Layer field

`Window` holds a `layer: u16` field for z-order and future transparency support. Applications that overlap will eventually be composited according to their layer.

### Delta-only preview rendering

During window resize, only the *new* border extensions are drawn on top of the existing frame — the original borders are not erased. This avoids a full redraw on every cursor move and gives a clear "ghost outline" preview of the new size.
