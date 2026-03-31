/// Subclasses the main Tauri window to intercept WM_MOVE and reposition
/// all terminal popup HWNDs immediately, eliminating the visual lag caused
/// by the frontend's debounced IPC round-trip.

use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::{
    Foundation::*,
    Graphics::Gdi::ClientToScreen,
    UI::WindowsAndMessaging::*,
};

static ORIG_WNDPROC: AtomicIsize = AtomicIsize::new(0);
static LAST_CLIENT_X: AtomicIsize = AtomicIsize::new(0);
static LAST_CLIENT_Y: AtomicIsize = AtomicIsize::new(0);

/// Install our subclass on the main Tauri HWND. Call once during setup.
pub unsafe fn subclass_main_window(hwnd_val: isize) {
    let hwnd = HWND(hwnd_val as *mut _);

    // Store initial client-area origin in screen coords (matches WM_MOVE lparam)
    let mut pt = POINT { x: 0, y: 0 };
    let _ = ClientToScreen(hwnd, &mut pt);
    LAST_CLIENT_X.store(pt.x as isize, Ordering::Relaxed);
    LAST_CLIENT_Y.store(pt.y as isize, Ordering::Relaxed);

    let old = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, main_subclass_proc as *const () as isize);
    ORIG_WNDPROC.store(old, Ordering::Relaxed);
}

unsafe extern "system" fn main_subclass_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    if msg == WM_MOVE {
        // lparam = (x, y) of client area origin in screen coords (for top-level windows)
        let new_x = (lparam.0 & 0xFFFF) as i16 as i32;
        let new_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
        let old_x = LAST_CLIENT_X.swap(new_x as isize, Ordering::Relaxed) as i32;
        let old_y = LAST_CLIENT_Y.swap(new_y as isize, Ordering::Relaxed) as i32;
        let dx = new_x - old_x;
        let dy = new_y - old_y;
        if dx != 0 || dy != 0 {
            reposition_owned_terminals(hwnd, dx, dy);
        }
    }

    let original: WNDPROC = std::mem::transmute(ORIG_WNDPROC.load(Ordering::Relaxed));
    CallWindowProcW(original, hwnd, msg, wparam, lparam)
}

/// Batch-reposition all VMUX_TERMINAL popups owned by `owner` by (dx, dy) pixels.
/// Uses DeferWindowPos for flicker-free atomic repositioning.
///
/// Walks the window list via GetWindow(GW_HWNDFIRST/GW_HWNDNEXT) instead of
/// EnumThreadWindows to avoid callback type issues with the windows crate.
unsafe fn reposition_owned_terminals(owner: HWND, dx: i32, dy: i32) {
    let mut terminals: Vec<HWND> = Vec::new();

    // Walk all top-level windows on the desktop
    let Ok(mut cur) = GetTopWindow(None) else { return };
    loop {
        // Reposition ALL visible windows owned by the main window
        // (terminal HWNDs + browser popup)
        if let Ok(win_owner) = GetWindow(cur, GW_OWNER) {
            if win_owner == owner && IsWindowVisible(cur).as_bool() {
                terminals.push(cur);
            }
        }
        match GetWindow(cur, GW_HWNDNEXT) {
            Ok(next) => cur = next,
            Err(_) => break,
        }
    }

    if terminals.is_empty() {
        return;
    }

    // Batch all moves atomically for flicker-free repositioning
    if let Ok(mut hdwp) = BeginDeferWindowPos(terminals.len() as i32) {
        for &term_hwnd in &terminals {
            let mut rect = RECT::default();
            if GetWindowRect(term_hwnd, &mut rect).is_ok() {
                if let Ok(new_hdwp) = DeferWindowPos(
                    hdwp, term_hwnd, None,
                    rect.left + dx, rect.top + dy,
                    0, 0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                ) {
                    hdwp = new_hdwp;
                }
            }
        }
        let _ = EndDeferWindowPos(hdwp);
    }
}
