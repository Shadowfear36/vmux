/// Translates Win32 virtual key codes into terminal input sequences.

use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Debug, Clone)]
pub enum InputEvent {
    Char(char),
    Sequence(Vec<u8>),
    Scroll(i32),
}

impl InputEvent {
    pub fn to_pty_bytes(&self) -> Vec<u8> {
        match self {
            InputEvent::Char(c) => {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).as_bytes().to_vec()
            }
            InputEvent::Sequence(seq) => seq.clone(),
            InputEvent::Scroll(lines) => {
                let seq = if *lines > 0 { b"\x1b[A" as &[u8] } else { b"\x1b[B" };
                seq.repeat(lines.unsigned_abs() as usize)
            }
        }
    }
}

/// Translate a WM_KEYDOWN virtual key + flags into a terminal InputEvent.
/// Returns None for printable keys (let WM_CHAR handle those).
pub fn translate_key(vk: u32, _flags: u32, _reserved: bool) -> Option<InputEvent> {
    let ctrl  = unsafe { (GetKeyState(VK_CONTROL.0 as i32) & 0x8000u16 as i16) != 0 };
    let shift = unsafe { (GetKeyState(VK_SHIFT.0 as i32)   & 0x8000u16 as i16) != 0 };

    let seq: &[u8] = match VIRTUAL_KEY(vk as u16) {
        // VK_RETURN handled by WM_CHAR (avoids double-send)
        VK_ESCAPE   => b"\x1b",
        VK_BACK     => b"\x7f",
        // Normal Tab handled by WM_CHAR; only intercept Shift+Tab here
        VK_TAB      => if shift { b"\x1b[Z" } else { return None },
        VK_UP       => if ctrl { b"\x1b[1;5A" } else if shift { b"\x1b[1;2A" } else { b"\x1b[A" },
        VK_DOWN     => if ctrl { b"\x1b[1;5B" } else if shift { b"\x1b[1;2B" } else { b"\x1b[B" },
        VK_RIGHT    => if ctrl { b"\x1b[1;5C" } else if shift { b"\x1b[1;2C" } else { b"\x1b[C" },
        VK_LEFT     => if ctrl { b"\x1b[1;5D" } else if shift { b"\x1b[1;2D" } else { b"\x1b[D" },
        VK_HOME     => b"\x1b[H",
        VK_END      => b"\x1b[F",
        VK_INSERT   => b"\x1b[2~",
        VK_DELETE   => b"\x1b[3~",
        VK_PRIOR    => b"\x1b[5~",
        VK_NEXT     => b"\x1b[6~",
        VK_F1       => b"\x1bOP",
        VK_F2       => b"\x1bOQ",
        VK_F3       => b"\x1bOR",
        VK_F4       => b"\x1bOS",
        VK_F5       => b"\x1b[15~",
        VK_F6       => b"\x1b[17~",
        VK_F7       => b"\x1b[18~",
        VK_F8       => b"\x1b[19~",
        VK_F9       => b"\x1b[20~",
        VK_F10      => b"\x1b[21~",
        VK_F11      => b"\x1b[23~",
        VK_F12      => b"\x1b[24~",
        _           => return None,
    };

    Some(InputEvent::Sequence(seq.to_vec()))
}
