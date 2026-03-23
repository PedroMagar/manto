# Manto

Manto is a terminal-driven desktop environment written in Rust. It provides floating windows, multiple desktops, a dock shell, detached terminal windows, window snapping, splitting, resizing, and keyboard-first navigation.

## How To Run

```bash
cargo run
```

## How To Use

Manto has a few main contexts:
- `Normal`: move the pointer and interact with windows, tabs, desktops, and the dock.
- `Typing`: type commands into the dock shell.
- `TerminalFocus`: type inside a detached terminal window.
- `Moving`: reposition the active window.
- `Resizing`: preview and apply a new size for the active window.

The dock shell lives on the bottom bar. Press `Space` or `Enter` on the `.> ` area to start typing.

## Shortcuts

### Global Window/Desktop Shortcuts

- `Ctrl+T`: open a new terminal window.
- `Ctrl+W`: close the active window.
- `Ctrl+F`: maximize or restore the active window.
- `Ctrl+N`: focus the next visible window.
- `Ctrl+P`: focus the previous visible window.
- `Ctrl+X`: minimize the active window.
- `Ctrl+D`: open or close the Start menu.
- `Ctrl+1`, `Ctrl+2`, `Ctrl+3`, `Ctrl+4`: move the active window to desktop 1-4 and follow it.
- `1`, `2`, `3`, `4`: switch to desktop 1-4.
- `Ctrl+Delete`: quit Manto.

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

## More Documentation

Architecture, portability notes, and terminal integration direction were moved to [ARCHITECTURE.md](ARCHITECTURE.md).
