/// Native Win32 child window for a terminal pane.
///
/// Keyboard input is handled DIRECTLY in the WndProc (WM_KEYDOWN / WM_CHAR).
/// This avoids the WebView2 focus problem entirely — the HWND owns keyboard
/// focus and writes keystrokes straight to the PTY.
///
/// Ctrl+A activates prefix mode (tmux-style). In prefix mode, the next key
/// is sent as a PrefixCommand to the frontend instead of to the PTY.
/// Ctrl+A Ctrl+A sends a literal Ctrl+A (0x01) to the PTY.

use anyhow::Result;
use windows::{
    core::w,
    Win32::{
        Foundation::*,
        Graphics::Gdi::{BeginPaint, ClientToScreen, EndPaint, HBRUSH, PAINTSTRUCT},
        System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        System::LibraryLoader::GetModuleHandleW,
        System::Memory::{GlobalLock, GlobalUnlock},
        UI::Input::KeyboardAndMouse::*,
        UI::WindowsAndMessaging::*,
    },
};
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tauri::AppHandle;

use super::input::InputEvent;

static CLASS_REGISTERED: OnceLock<()> = OnceLock::new();

/// Global prefix mode flag shared across all terminal HWNDs.
static PREFIX_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Data stored in GWLP_USERDATA for each terminal HWND.
struct WndProcData {
    msg_tx: mpsc::UnboundedSender<WindowMessage>,
    owner_hwnd: HWND,
    pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum WindowMessage {
    Input(InputEvent),
    Resize(u32, u32),
    Paste(String),
    Clicked,
    Close,
    /// Prefix-mode command key (e.g. 'c' for new tab, 'n' for next tab).
    PrefixCommand(char),
    /// Prefix mode was activated — frontend should show PREFIX badge.
    PrefixActivated,
    /// Prefix mode was deactivated.
    PrefixDeactivated,
}

pub struct TerminalWindow {
    pub hwnd: HWND,
    owner_hwnd: isize,
    #[allow(dead_code)]
    pub msg_tx: mpsc::UnboundedSender<WindowMessage>,
}

impl TerminalWindow {
    pub async fn create_on_main_thread(
        app: &AppHandle,
        parent_hwnd: isize,
        x: i32, y: i32, width: i32, height: i32,
        pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<WindowMessage>)> {
        let (tx, rx) = mpsc::unbounded_channel::<WindowMessage>();
        let (hwnd_tx, hwnd_rx) = tokio::sync::oneshot::channel::<isize>();

        let tx_clone = tx.clone();

        app.run_on_main_thread(move || {
            let hwnd_val = unsafe { create_window(parent_hwnd, x, y, width, height, tx_clone, pty_writer) }
                .map(|h| h.0 as isize)
                .unwrap_or(0);
            let _ = hwnd_tx.send(hwnd_val);
        }).map_err(|e| anyhow::anyhow!("run_on_main_thread failed: {e:?}"))?;

        let hwnd_val = hwnd_rx.await
            .map_err(|_| anyhow::anyhow!("HWND sender dropped before sending"))?;

        if hwnd_val == 0 {
            return Err(anyhow::anyhow!("CreateWindowExW returned null HWND"));
        }

        let hwnd = HWND(hwnd_val as *mut _);
        Ok((TerminalWindow { hwnd, owner_hwnd: parent_hwnd, msg_tx: tx }, rx))
    }

    pub fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            let owner = HWND(self.owner_hwnd as *mut _);
            let mut pt = POINT { x, y };
            let _ = ClientToScreen(owner, &mut pt);
            let _ = SetWindowPos(
                self.hwnd, None, pt.x, pt.y, width, height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    pub fn focus(&self) {
        unsafe { let _ = SetForegroundWindow(self.hwnd); }
    }

    #[allow(dead_code)]
    pub fn hide(&self) { unsafe { let _ = ShowWindow(self.hwnd, SW_HIDE); } }
    #[allow(dead_code)]
    pub fn show(&self) { unsafe { let _ = ShowWindow(self.hwnd, SW_SHOW); } }
    pub fn hwnd_isize(&self) -> isize { self.hwnd.0 as isize }
}

impl Drop for TerminalWindow {
    fn drop(&mut self) {
        unsafe { let _ = DestroyWindow(self.hwnd); }
    }
}

unsafe impl Send for TerminalWindow {}
unsafe impl Sync for TerminalWindow {}

// ─── Internal ─────────────────────────────────────────────────────────────────

unsafe fn create_window(
    parent_hwnd: isize,
    x: i32, y: i32, width: i32, height: i32,
    msg_tx: mpsc::UnboundedSender<WindowMessage>,
    pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
) -> Result<HWND> {
    register_window_class()?;

    let data = WndProcData {
        msg_tx,
        owner_hwnd: HWND(parent_hwnd as *mut _),
        pty_writer,
    };
    let data_ptr = Box::into_raw(Box::new(data)) as isize;
    let hinstance = GetModuleHandleW(None)?;

    let owner = HWND(parent_hwnd as *mut _);
    let mut pt = POINT { x, y };
    let _ = ClientToScreen(owner, &mut pt);

    // WS_POPUP — sits above WebView2's DirectComposition layer.
    // WS_EX_TOOLWINDOW — hides from taskbar and Alt-Tab.
    // NO WS_EX_NOACTIVATE — HWND takes keyboard focus on click.
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        w!("VMUX_TERMINAL"),
        w!(""),
        WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
        pt.x, pt.y, width, height,
        Some(owner),
        None,
        Some(HINSTANCE(hinstance.0)),
        Some(data_ptr as *const _),
    )?;

    Ok(hwnd)
}

fn register_window_class() -> Result<()> {
    CLASS_REGISTERED.get_or_init(|| unsafe {
        let hinstance = GetModuleHandleW(None).expect("GetModuleHandleW");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(terminal_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: w!("VMUX_TERMINAL"),
            hCursor: LoadCursorW(None, IDC_IBEAM).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut() as _),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
    Ok(())
}

// ─── WndProc ──────────────────────────────────────────────────────────────────

unsafe extern "system" fn terminal_wnd_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam.0 as *const CREATESTRUCTW;
            if !create.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_SIZE => {
            let w = (lparam.0 & 0xFFFF) as u32;
            let h = ((lparam.0 >> 16) & 0xFFFF) as u32;
            send_msg(hwnd, WindowMessage::Resize(w, h));
            LRESULT(0)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _hdc = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        // ── Keyboard input (native, bypasses WebView2 entirely) ──────────
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = VIRTUAL_KEY(wparam.0 as u16);
            let ctrl  = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;

            // ── Ctrl+A: prefix system ────────────────────────────────────
            if ctrl && vk == VK_A {
                if PREFIX_ACTIVE.load(Ordering::Relaxed) {
                    // Ctrl+A Ctrl+A → send literal Ctrl+A to PTY
                    PREFIX_ACTIVE.store(false, Ordering::Relaxed);
                    send_msg(hwnd, WindowMessage::PrefixDeactivated);
                    write_pty(hwnd, &[0x01]);
                } else {
                    PREFIX_ACTIVE.store(true, Ordering::Relaxed);
                    send_msg(hwnd, WindowMessage::PrefixActivated);
                }
                return LRESULT(0);
            }

            // ── In prefix mode, intercept the command key ────────────────
            if PREFIX_ACTIVE.load(Ordering::Relaxed) {
                PREFIX_ACTIVE.store(false, Ordering::Relaxed);
                send_msg(hwnd, WindowMessage::PrefixDeactivated);
                // Map VK to character for the command
                if let Some(ch) = vk_to_command_char(vk) {
                    send_msg(hwnd, WindowMessage::PrefixCommand(ch));
                }
                return LRESULT(0);
            }

            // ── Ctrl+V: paste from clipboard ─────────────────────────────
            if ctrl && vk == VK_V {
                if let Some(text) = read_clipboard(hwnd) {
                    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
                    write_pty(hwnd, normalized.as_bytes());
                }
                return LRESULT(0);
            }

            // ── Ctrl+key sequences (Ctrl+C, Ctrl+D, etc.) ───────────────
            if ctrl && vk.0 >= 0x41 && vk.0 <= 0x5A {
                // A=0x41..Z=0x5A → control codes 1..26
                let code = (vk.0 - 0x40) as u8;
                write_pty(hwnd, &[code]);
                return LRESULT(0);
            }
            // Ctrl+[ = ESC, Ctrl+\ = FS, Ctrl+] = GS
            if ctrl {
                match vk {
                    VK_OEM_4 => { write_pty(hwnd, &[0x1b]); return LRESULT(0); } // [
                    VK_OEM_5 => { write_pty(hwnd, &[0x1c]); return LRESULT(0); } // \
                    VK_OEM_6 => { write_pty(hwnd, &[0x1d]); return LRESULT(0); } // ]
                    _ => {}
                }
            }

            // ── Special keys → VT escape sequences ──────────────────────
            let seq: Option<&[u8]> = match vk {
                VK_RETURN    => Some(b"\r"),
                VK_BACK      => Some(b"\x7f"),
                VK_ESCAPE    => Some(b"\x1b"),
                VK_TAB       => if shift { Some(b"\x1b[Z") } else { Some(b"\t") },
                VK_DELETE    => Some(b"\x1b[3~"),
                VK_INSERT    => Some(b"\x1b[2~"),
                VK_HOME      => Some(b"\x1b[H"),
                VK_END       => Some(b"\x1b[F"),
                VK_PRIOR     => Some(b"\x1b[5~"),  // PageUp
                VK_NEXT      => Some(b"\x1b[6~"),  // PageDown
                VK_UP    => Some(if ctrl { b"\x1b[1;5A" } else if shift { b"\x1b[1;2A" } else { b"\x1b[A" }),
                VK_DOWN  => Some(if ctrl { b"\x1b[1;5B" } else if shift { b"\x1b[1;2B" } else { b"\x1b[B" }),
                VK_RIGHT => Some(if ctrl { b"\x1b[1;5C" } else if shift { b"\x1b[1;2C" } else { b"\x1b[C" }),
                VK_LEFT  => Some(if ctrl { b"\x1b[1;5D" } else if shift { b"\x1b[1;2D" } else { b"\x1b[D" }),
                VK_F1  => Some(b"\x1bOP"),
                VK_F2  => Some(b"\x1bOQ"),
                VK_F3  => Some(b"\x1bOR"),
                VK_F4  => Some(b"\x1bOS"),
                VK_F5  => Some(b"\x1b[15~"),
                VK_F6  => Some(b"\x1b[17~"),
                VK_F7  => Some(b"\x1b[18~"),
                VK_F8  => Some(b"\x1b[19~"),
                VK_F9  => Some(b"\x1b[20~"),
                VK_F10 => Some(b"\x1b[21~"),
                VK_F11 => Some(b"\x1b[23~"),
                VK_F12 => Some(b"\x1b[24~"),
                _ => None,
            };
            if let Some(bytes) = seq {
                write_pty(hwnd, bytes);
                return LRESULT(0);
            }

            // Let DefWindowProcW generate WM_CHAR for printable keys
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CHAR => {
            let ch = wparam.0 as u32;
            // Skip control characters already handled in WM_KEYDOWN
            if ch < 32 { return LRESULT(0); }
            // Encode Unicode character as UTF-8 and write to PTY
            if let Some(c) = char::from_u32(ch) {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                write_pty(hwnd, encoded.as_bytes());
            }
            LRESULT(0)
        }

        // ── Mouse ────────────────────────────────────────────────────────
        WM_LBUTTONDOWN => {
            // Take keyboard focus on click
            let _ = SetFocus(Some(hwnd));
            send_msg(hwnd, WindowMessage::Clicked);
            LRESULT(0)
        }

        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) as i16) / 120;
            send_msg(hwnd, WindowMessage::Input(InputEvent::Scroll(delta as i32)));
            LRESULT(0)
        }

        WM_DESTROY => {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WndProcData;
            if !ptr.is_null() {
                drop(Box::from_raw(ptr));
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

unsafe fn get_data(hwnd: HWND) -> *const WndProcData {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WndProcData
}

unsafe fn send_msg(hwnd: HWND, msg: WindowMessage) {
    let ptr = get_data(hwnd);
    if !ptr.is_null() {
        let _ = (*ptr).msg_tx.send(msg);
    }
}

unsafe fn write_pty(hwnd: HWND, data: &[u8]) {
    let ptr = get_data(hwnd);
    if !ptr.is_null() {
        // Use try_lock to avoid blocking the main thread's message pump.
        // If the lock is held (e.g. by EventProxy writing a PtyWrite response),
        // we skip this keystroke rather than freeze the UI.
        if let Ok(mut w) = (*ptr).pty_writer.try_lock() {
            let _ = w.write_all(data);
        }
    }
}

/// Map a virtual key code to a command character for the prefix system.
fn vk_to_command_char(vk: VIRTUAL_KEY) -> Option<char> {
    match vk.0 {
        0x41..=0x5A => Some((vk.0 as u8 + 32) as char), // A-Z → a-z
        0x30..=0x39 => Some((vk.0 as u8) as char),       // 0-9
        _ => None,
    }
}

/// Read UTF-16 text from the Win32 clipboard.
unsafe fn read_clipboard(hwnd: HWND) -> Option<String> {
    const CF_UNICODETEXT: u32 = 13;
    OpenClipboard(Some(hwnd)).ok()?;
    let result = (|| -> Option<String> {
        let h = GetClipboardData(CF_UNICODETEXT).ok()?;
        let hglobal = HGLOBAL(h.0);
        let ptr = GlobalLock(hglobal) as *const u16;
        if ptr.is_null() { return None; }
        let mut len = 0usize;
        while *ptr.add(len) != 0 { len += 1; }
        let wide = std::slice::from_raw_parts(ptr, len);
        let _ = GlobalUnlock(hglobal);
        Some(String::from_utf16_lossy(wide))
    })();
    let _ = CloseClipboard();
    result
}
