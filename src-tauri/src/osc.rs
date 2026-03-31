/// Parses OSC (Operating System Command) escape sequences from terminal output.
/// Supports OSC 9, 99, and 777 which are used by agents/tools to send notifications.
///
/// OSC 9:    \x1b]9;<message>\x07  (ConEmu notification)
/// OSC 99:   \x1b]99;i=1:d=1;<message>\x07  (systemd/notify style)
/// OSC 777:  \x1b]777;notify;<title>;<body>\x07  (rxvt-unicode)
///
/// vmux browser control (custom):
/// \x1b]vmux;browser-open;<url>\x07     — open browser pane and navigate to URL
/// \x1b]vmux;browser-navigate;<url>\x07 — navigate existing browser to URL
/// \x1b]vmux;browser-close\x07          — close browser pane
/// \x1b]vmux;browser-eval;<js>\x07      — evaluate JavaScript in browser
use regex::Regex;

/// Parsed result from terminal OSC output.
#[derive(Debug, Clone)]
pub enum OscAction {
    Notification(String),
    BrowserOpen(String),
    BrowserNavigate(String),
    BrowserClose,
    BrowserEval(String),
    /// OSC 7: shell reported its current working directory
    CwdChanged(String),
}

pub struct OscParser {
    osc9_re: Regex,
    osc99_re: Regex,
    osc777_re: Regex,
    vmux_re: Regex,
    osc7_re: Regex,
}

impl OscParser {
    pub fn new() -> Self {
        OscParser {
            osc9_re: Regex::new(r"\x1b\]9;([^\x07\x1b]*)\x07").unwrap(),
            osc99_re: Regex::new(r"\x1b\]99;[^;]*;([^\x07\x1b]*)\x07").unwrap(),
            osc777_re: Regex::new(r"\x1b\]777;notify;([^\x07;]*);?([^\x07]*)\x07").unwrap(),
            vmux_re: Regex::new(r"\x1b\]vmux;([^\x07\x1b]*)\x07").unwrap(),
            // OSC 7: \x1b]7;file://host/path\x07 or \x1b]7;/path\x07
            osc7_re: Regex::new(r"\x1b\]7;(?:file://[^/]*)?(/?[^\x07\x1b]*)\x07").unwrap(),
        }
    }

    /// Returns the notification message if an OSC notification escape sequence is found.
    pub fn parse(&mut self, data: &str) -> Option<String> {
        if let Some(caps) = self.osc9_re.captures(data) {
            return Some(caps[1].to_string());
        }
        if let Some(caps) = self.osc99_re.captures(data) {
            return Some(caps[1].to_string());
        }
        if let Some(caps) = self.osc777_re.captures(data) {
            let title = &caps[1];
            let body = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if body.is_empty() {
                return Some(title.to_string());
            }
            return Some(format!("{}: {}", title, body));
        }
        None
    }

    /// Parse all OSC actions (notifications + vmux browser commands).
    pub fn parse_actions(&mut self, data: &str) -> Vec<OscAction> {
        let mut actions = Vec::new();

        // Standard notifications
        if let Some(msg) = self.parse(data) {
            actions.push(OscAction::Notification(msg));
        }

        // OSC 7: current working directory
        if let Some(caps) = self.osc7_re.captures(data) {
            let path = caps[1].to_string();
            // Decode percent-encoded characters (e.g. %20 → space)
            let decoded = percent_decode(&path);
            if !decoded.is_empty() {
                actions.push(OscAction::CwdChanged(decoded));
            }
        }

        // vmux custom commands
        for caps in self.vmux_re.captures_iter(data) {
            let payload = &caps[1];
            let parts: Vec<&str> = payload.splitn(2, ';').collect();
            match parts.get(0).copied() {
                Some("browser-open") => {
                    if let Some(url) = parts.get(1) {
                        actions.push(OscAction::BrowserOpen(url.to_string()));
                    }
                }
                Some("browser-navigate") => {
                    if let Some(url) = parts.get(1) {
                        actions.push(OscAction::BrowserNavigate(url.to_string()));
                    }
                }
                Some("browser-close") => {
                    actions.push(OscAction::BrowserClose);
                }
                Some("browser-eval") => {
                    if let Some(js) = parts.get(1) {
                        actions.push(OscAction::BrowserEval(js.to_string()));
                    }
                }
                _ => {}
            }
        }

        actions
    }
}

/// Decode percent-encoded characters in a file:// URL path.
fn percent_decode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                &s[i + 1..i + 3], 16,
            ) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}
