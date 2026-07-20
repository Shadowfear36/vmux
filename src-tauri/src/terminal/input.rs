/// Non-keyboard input events forwarded from the Win32 WndProc.
/// Keyboard input is translated to PTY bytes directly in `window.rs`'s
/// WndProc (WM_KEYDOWN / WM_CHAR) — this only carries other input kinds.

#[derive(Debug, Clone)]
pub enum InputEvent {
    Scroll(i32),
}
