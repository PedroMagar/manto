# Manto

Manto is a terminal-driven desktop environment written in Rust. It provides floating windows, multiple desktops, a dock shell, detached terminal windows, window snapping, splitting, resizing, keyboard-first navigation and **mouse support**. The desktop is rendered with damage-tracking (no full redraw per frame), sessions persist between runs and the shortcuts/theme are configurable.

> **AI-generated code**: this entire codebase was written with extensive use of
> AI assistance — every file, feature and fix in this repository was produced
> by AI tooling.

## License

MIT — see [LICENSE](LICENSE).

## How To Run

```bash
cargo run
```

## How To Use

Manto has a few main contexts:
- `Normal`: move the pointer and interact with windows, tabs, desktops, and the dock.
- `Typing`: type commands into the dock shell.
- `TerminalFocus`: type inside a detached terminal window; interactive apps (`#i python`, `vim`, ...) run through a full terminal emulator.
- `Moving`: reposition the active window.
- `Resizing`: preview and apply a new size for the active window.

The dock shell lives on the bottom bar. Press `Space` or `Enter` on the `.> ` area to start typing.

## Mouse

Mouse is **enabled by default** and can be toggled with **`Alt+M`** (or
`"mouse"` in `~/.manto/config.json`). While enabled:

- **Hover** over the window chrome, tabs, scrollbars, the Start button and the
  desktop buttons highlights the action points; moving inside a window just
  moves the pointer.
- **Left click** activates what's under the pointer (focus, restore from tab,
  switch desktop, toggle Start, enter a terminal, scroll a bar). Clicking the
  dock or terminal *`.> `* input box enters typing/focus mode and keeps it
  until you `Esc`/`End` or click elsewhere.
- **Left click-and-drag** over any window or desktop text starts a screen text
  selection that follows the pointer and stays after you release
  (`Enter`/`Ctrl+C` copies, `Esc` clears). A plain click clears any stale
  selection. Dragging the chrome still moves/resizes windows and scroll pages.
- **Drag** a title bar to move a window and the bottom-right corner to resize.
- **Wheel** scrolls the terminal under the pointer, the dock panel or the
  minimized-window rail.
- **Double-click** a title bar to maximize/restore.
- **Right click** focuses (raises) the window under the pointer.
- Inside an **interactive terminal**, pointer events are forwarded to the app
  (SGR); clicking anywhere in the app body enters it, clicking outside leaves
  it. Click-drag there goes to the app (its own selection), not Manto's.
- Free screen selection is also available with `Shift+arrows`; `Enter`/`Ctrl+C`
  copies the current box. `Alt+M` is handy when a console app grabs clicks
  you want for yourself.

## Shortcuts

Shortcuts marked *configurable* can be remapped in `~/.manto/config.json`
under `"shortcuts"` (e.g. `{ "terminal": "ctrl+t", "mouse": "alt+m" }`).

### Global Window/Desktop Shortcuts

- `Ctrl+T` *configurable*: open a new terminal window.
- `Ctrl+W` *configurable*: close the active window.
- `Ctrl+F` *configurable*: maximize or restore the active window.
- `Ctrl+N` *configurable*: focus the next visible window.
- `Ctrl+P` *configurable*: focus the previous visible window.
- `Ctrl+X` *configurable*: minimize the active window.
- `Ctrl+D` *configurable*: open or close the Start menu.
- `Ctrl+H` / `F1` *configurable*: open or close the help window, a crib sheet
  with all usage tips and shortcuts (scroll with arrows/Page keys or the
  wheel; `Esc` or another `F1`/`Ctrl+H` closes). `Ctrl+H` is Windows-only —
  the Unix console encodes it as Backspace, so there use `F1`.
- `Alt+M` *configurable*: toggle mouse input (default on).
- `Ctrl+1`, `Ctrl+2`, `Ctrl+3`, `Ctrl+4`: move the active window to desktop 1-4 and follow it.
- `1`, `2`, `3`, `4`: switch to desktop 1-4.
- `Ctrl+Delete` *configurable*: quit Manto (also saves the session).

### Window Snap And Split

- `Alt+Left`: snap the active window to the left half.
- `Alt+Right`: snap the active window to the right half.
- `Alt+Down`: snap the active window to the bottom half.
- `Alt+Up`: snap the active window to the top half.
- `Alt+Up` again on a window already in the top half: maximize it.
- `Alt+Up` again on that maximized window: restore it to the top half.
- Hold an orthogonal arrow while using `Alt+Arrow` to snap to a quarter:
  `Alt+Left+Up`, `Alt+Right+Up`, `Alt+Left+Down`, `Alt+Right+Down`.
- `Alt+V`: split the active terminal vertically and create a new terminal on the right.
- `Alt+H`: split the active terminal horizontally and create a new terminal below.
- `Alt+R`: enter resize mode for the active window.

### Normal Mode

- `Up`, `Down`, `Left`, `Right`: move the pointer.
- `Home`: move the pointer to the dock shell input.
- `:`: move the pointer to the dock shell input and enter typing mode.
- `Space` or `Enter`: activate what is under the pointer.

### Dock Shell (`Typing`)

- `Esc` or `End`: leave dock typing mode.
- `Ctrl+Enter`: detach the dock shell into a floating terminal window.
- `PageUp`, `PageDown`: scroll the dock command panel.
- `Up`, `Down`: browse command history.
- `Left`, `Right`: move the text cursor.
- `Tab`: autocomplete commands and paths.
- `Enter`: run the current command.
- `Backspace`: delete before the cursor.
- `Delete`: delete at the cursor.

### Detached Terminal (`TerminalFocus`)

- `Esc` or `End`: leave terminal focus mode.
- `PageUp`, `PageDown`: scroll that terminal's command history panel.
- `Up`, `Down`: browse command history.
- `Left`, `Right`: move the text cursor.
- `Tab`: autocomplete commands and paths.
- `Enter`: run the current command.
- `Backspace`: delete before the cursor.
- `Delete`: delete at the cursor.

### Moving Mode

- `Up`, `Down`, `Left`, `Right`: move the window preview.
- `Space` or `Enter`: confirm the new position.

### Resizing Mode

- `Up`, `Down`, `Left`, `Right`: change the resize preview with the pointer.
- `Space` or `Enter`: apply the previewed size and exit resize mode.
- `Esc`: cancel and exit resize mode.

Numeric resize editing inside resize mode:
- `X` or `H`: select width editing.
- `Y` or `V`: select height editing.
- `+`: add a value.
- `-`: subtract a value.
- `=`: set an exact value.
- Digits: type the amount.
- `Enter`: apply the typed numeric change to the preview.
- `Backspace`: erase the last typed digit.
- `Esc`: cancel the numeric edit; if no numeric edit is active, it exits resize mode.
- `Space`: ignored while typing the numeric edit.

## Interactive Terminals

Terminals opened with **`#i`** run the program through a full terminal
emulator: `#i vim` opens vim, a bare `#i` opens the system shell, and known
interactive apps (python, node, vim, nano, top, less, bash/pwsh/cmd, ...) open
that way automatically. Inside:

- Keys are forwarded raw; `Esc`/`End` return to the desktop.
- `PageUp`/`PageDown` scroll Manto's scrollback.
- `Ctrl+C` interrupts, `Ctrl+V` pastes, `Ctrl+D`/`Ctrl+Z` send EOF.
- Pointer events are forwarded to the app (SGR); click outside to leave.
- On hosts without a usable PTY/ConPTY (piped fallback) Manto echoes locally,
  recalls a small local history with `Up`/`Down`, ignores navigation keys that
  a pipe can't interpret and auto-adds `-i` to bare `python`/`python2`/
  `python3` so REPLs keep prompting.

## Configuration

`~/.manto/config.json` (optional):

```json
{
  "theme": 1,
  "shortcuts": { "terminal": "ctrl+t", "mouse": "alt+m" }
}
```

- `theme`: 0 (none), 1 (top border) or 2 (full border).
- `shortcuts`: remap desktop actions — `terminal`, `close`, `maximize`,
  `start_menu`, `help`, `split_vertical`, `split_horizontal`, `minimize`,
  `focus_next`, `focus_prev`, `resize`, `mouse`, `quit` (values like
  `"ctrl+t"` / `"alt+v"` / `"f1"` / `"enter"`).

## Persistence

On quit (`Ctrl+Delete`) the desktop saves window positions/sizes, titles,
working directories and the active desktop to `~/.manto/session.json`; the
next start re-creates the terminals with the same layout (shell sessions are
host processes, so only the layout survives).

## More Documentation

Architecture, portability notes, terminal integration and rendering model were moved to [ARCHITECTURE.md](ARCHITECTURE.md).
