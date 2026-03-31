use serde::{Deserialize, Serialize};

/// Color theme for terminal rendering.
/// Colors are [r, g, b, a] u8 values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub cursor: [u8; 4],
    pub selection_bg: [u8; 4],
    /// 16 ANSI colors: black, red, green, yellow, blue, magenta, cyan, white,
    ///                  bright variants of each
    pub ansi: [[u8; 4]; 16],
    /// Active pane border (for the tmux-style focus indicator)
    pub border_active: [u8; 4],
    /// Inactive pane border
    pub border_inactive: [u8; 4],
    /// Sidebar background
    pub sidebar_bg: [u8; 4],
    /// Notification ring color (the cmux-style agent alert)
    pub notification_ring: [u8; 4],
}

impl Theme {
    /// Tokyo Night — popular dark theme for terminals
    pub fn tokyo_night() -> Self {
        Theme {
            background:       [26,  27,  38, 255],
            foreground:       [169, 177, 214, 255],
            cursor:           [192, 202, 245, 255],
            selection_bg:     [40,  46,  77, 180],
            border_active:    [122, 162, 247, 255],
            border_inactive:  [41,  46,  66, 255],
            sidebar_bg:       [22,  22,  30, 255],
            notification_ring:[86,  95, 137, 255],
            ansi: [
                [26,  27,  38, 255],  // black
                [247, 118, 142, 255], // red
                [115, 218, 202, 255], // green
                [224, 175, 104, 255], // yellow
                [122, 162, 247, 255], // blue
                [187, 154, 247, 255], // magenta
                [42,  195, 222, 255], // cyan
                [169, 177, 214, 255], // white
                [65,  72, 104, 255],  // bright black
                [255, 117, 127, 255], // bright red
                [158, 206, 106, 255], // bright green
                [224, 175, 104, 255], // bright yellow
                [122, 162, 247, 255], // bright blue
                [187, 154, 247, 255], // bright magenta
                [42,  195, 222, 255], // bright cyan
                [192, 202, 245, 255], // bright white
            ],
        }
    }

    /// Catppuccin Mocha
    pub fn catppuccin_mocha() -> Self {
        Theme {
            background:       [30,  30,  46, 255],
            foreground:       [205, 214, 244, 255],
            cursor:           [243, 139, 168, 255],
            selection_bg:     [69,  71,  90, 200],
            border_active:    [137, 180, 250, 255],
            border_inactive:  [49,  50,  68, 255],
            sidebar_bg:       [24,  24,  37, 255],
            notification_ring:[166, 227, 161, 255],
            ansi: [
                [69,  71,  90, 255],  // black
                [243, 139, 168, 255], // red
                [166, 227, 161, 255], // green
                [249, 226, 175, 255], // yellow
                [137, 180, 250, 255], // blue
                [203, 166, 247, 255], // magenta
                [148, 226, 213, 255], // cyan
                [166, 173, 200, 255], // white
                [88,  91, 112, 255],  // bright black
                [243, 139, 168, 255], // bright red
                [166, 227, 161, 255], // bright green
                [249, 226, 175, 255], // bright yellow
                [137, 180, 250, 255], // bright blue
                [203, 166, 247, 255], // bright magenta
                [148, 226, 213, 255], // bright cyan
                [205, 214, 244, 255], // bright white
            ],
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::tokyo_night()
    }
}
