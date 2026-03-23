# Architecture

## Design Goals

Manto prioritizes portability, low dependency surface, and clear separation between UI logic and host-specific integration. The long-term goal is to keep the project viable both on mainstream operating systems and on a future custom OS.

## Portability Over Dependencies

The project avoids third-party terminal crates such as `crossterm` and works directly with ANSI / VT100 output plus a thin host abstraction layer. This keeps the runtime model simple and reduces friction for future ports.

## OS Isolation Layer

Everything that depends on the host OS is concentrated in `os.rs`:

- `Writer`: output abstraction
- `Clock`: time abstraction
- `Key`: keyboard event abstraction
- platform-specific modules for raw mode, terminal size, polling, and key decoding

The intended rule is: when porting Manto to another OS, most of the change should happen in `os.rs`.

## ANSI Layer

`ansi.rs` emits ANSI / VT100 control sequences through `std::io::Write` and does not depend on platform conditionals. This keeps rendering logic separate from OS handling.

## Application And Window Separation

`Application` represents the logical app state. `Window` represents the visible frame and geometry. This separation allows the same application to be windowed, minimized, maximized, or eventually represented in other forms without mixing presentation and app state.

Current display states are centered around:

- `Windowed(Window)`
- `Minimized(Window)`
- `Maximized { display, saved }`

## Layering

Each `Window` carries a `layer` field for z-order and future compositing use. The current stacking model is still mostly vector-order based, but the layer field preserves a path for richer composition later.

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

## Suggested Backend Interface

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

## Suggested Phases

### Phase 1: Command Runner

- run full commands
- capture stdout / stderr
- show line-based output in the internal terminal
- no persistent interactive shell yet

### Phase 2: Real Shell Backend

- start a shell through PTY / ConPTY
- forward raw key input
- read raw output back
- let the shell handle history and completion

### Phase 3: Richer Terminal Behavior

- proper resize handling
- full-screen terminal apps
- better scrollback
- colors, alternate screen, and cursor behavior

## Start Menu Direction

The Start menu should likely be driven by a declarative manifest before moving to arbitrary scripting.

Useful fields:

- `label`
- `kind`
- `command`
- `args`
- `cwd`
- `desktop`

That keeps it customizable without coupling it too early to an embedded scripting engine.
