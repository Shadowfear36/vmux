/// Shell profile detection for Windows.
/// Discovers cmd.exe, PowerShell 5/7, and Git Bash.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellProfile {
    /// Short identifier, e.g. "cmd", "powershell", "pwsh", "gitbash"
    pub id: String,
    /// Display name shown in UI
    pub name: String,
    /// Absolute path to the executable
    pub path: String,
    /// Extra arguments passed after the executable
    pub args: Vec<String>,
    /// Extra environment variables (name, value)
    pub env: Vec<(String, String)>,
}

impl ShellProfile {
    fn new(id: &str, name: &str, path: &str, args: &[&str], env: &[(&str, &str)]) -> Self {
        ShellProfile {
            id: id.into(),
            name: name.into(),
            path: path.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }
}

/// Return all detected shell profiles, in preference order.
/// Always includes cmd.exe and powershell.exe; others only if found on disk.
pub fn detect_shells() -> Vec<ShellProfile> {
    let mut shells = Vec::new();

    // ── cmd.exe ───────────────────────────────────────────────────────────────
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let cmd_path = format!(r"{}\System32\cmd.exe", sysroot);
    shells.push(ShellProfile::new(
        "cmd", "Command Prompt",
        &cmd_path,
        &[],
        &[("TERM", "dumb")],
    ));

    // ── PowerShell 5 (inbox, always present on Win10+) ────────────────────────
    let ps5_path = format!(
        r"{}\System32\WindowsPowerShell\v1.0\powershell.exe",
        sysroot
    );
    if Path::new(&ps5_path).exists() {
        shells.push(ShellProfile::new(
            "powershell", "Windows PowerShell",
            &ps5_path,
            &["-NoLogo"],
            &[("TERM", "xterm-256color")],
        ));
    }

    // ── PowerShell 7 (pwsh) — check common install paths ─────────────────────
    let prog_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    let pwsh_candidates = [
        format!(r"{}\PowerShell\7\pwsh.exe", prog_files),
        format!(r"{}\PowerShell\pwsh.exe", prog_files),
        // Also check x86 variant
        std::env::var("ProgramFiles(x86)")
            .map(|p| format!(r"{}\PowerShell\7\pwsh.exe", p))
            .unwrap_or_default(),
        // User-local install via winget / MSI
        std::env::var("LOCALAPPDATA")
            .map(|p| format!(r"{}\Microsoft\PowerShell\7\pwsh.exe", p))
            .unwrap_or_default(),
    ];
    for path in &pwsh_candidates {
        if !path.is_empty() && Path::new(path).exists() {
            shells.push(ShellProfile::new(
                "pwsh", "PowerShell 7",
                path,
                &["-NoLogo"],
                &[("TERM", "xterm-256color")],
            ));
            break;
        }
    }

    // Also check PATH for pwsh
    if !shells.iter().any(|s| s.id == "pwsh") {
        if let Some(path) = which("pwsh.exe") {
            shells.push(ShellProfile::new(
                "pwsh", "PowerShell 7",
                &path,
                &["-NoLogo"],
                &[("TERM", "xterm-256color")],
            ));
        }
    }

    // ── Git Bash ──────────────────────────────────────────────────────────────
    // Git for Windows installs bash at Git\bin\bash.exe (the MSYS2 bash wrapper)
    // and the actual bash at Git\usr\bin\bash.exe.
    // Prefer the bin\bash.exe launcher — it sets up the Git environment properly.
    let git_candidates = [
        format!(r"{}\Git\bin\bash.exe", prog_files),
        format!(r"{}\Git\usr\bin\bash.exe", prog_files),
        std::env::var("ProgramFiles(x86)")
            .map(|p| format!(r"{}\Git\bin\bash.exe", p))
            .unwrap_or_default(),
        // Scoop
        std::env::var("USERPROFILE")
            .map(|p| format!(r"{}\scoop\apps\git\current\bin\bash.exe", p))
            .unwrap_or_default(),
        // winget default location
        format!(r"{}\Git\bin\bash.exe", prog_files),
    ];

    // Also check registry: HKLM\SOFTWARE\GitForWindows -> InstallPath
    let registry_git = read_git_install_path_from_registry();

    let mut all_git_candidates: Vec<String> = Vec::new();
    if let Some(ref git_root) = registry_git {
        all_git_candidates.push(format!(r"{}\bin\bash.exe", git_root));
        all_git_candidates.push(format!(r"{}\usr\bin\bash.exe", git_root));
    }
    all_git_candidates.extend(git_candidates.iter().filter(|s| !s.is_empty()).cloned());

    for path in &all_git_candidates {
        if !path.is_empty() && Path::new(path).exists() {
            shells.push(ShellProfile::new(
                "gitbash", "Git Bash",
                path,
                &["--login", "-i"],
                &[
                    ("TERM", "xterm-256color"),
                    ("MSYSTEM", "MINGW64"),
                ],
            ));
            break;
        }
    }

    // PATH fallback for bash
    if !shells.iter().any(|s| s.id == "gitbash") {
        if let Some(path) = which("bash.exe") {
            // Only treat a bash on PATH as Git Bash if it looks like it's from Git for Windows
            if path.to_lowercase().contains("git") {
                shells.push(ShellProfile::new(
                    "gitbash", "Git Bash",
                    &path,
                    &["--login", "-i"],
                    &[
                        ("TERM", "xterm-256color"),
                        ("MSYSTEM", "MINGW64"),
                    ],
                ));
            }
        }
    }

    shells
}

/// Find `exe_name` on PATH, return the full path if found.
pub fn which(exe_name: &str) -> Option<String> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(';') {
        let candidate = format!(r"{}\{}", dir.trim(), exe_name);
        if Path::new(&candidate).exists() {
            return Some(candidate);
        }
    }
    None
}

/// Read the Git for Windows install path from the registry.
fn read_git_install_path_from_registry() -> Option<String> {
    use windows::Win32::System::Registry::*;
    use windows::core::PCWSTR;

    let key_path: Vec<u16> = "SOFTWARE\\GitForWindows\0"
        .encode_utf16().collect();
    let value_name: Vec<u16> = "InstallPath\0"
        .encode_utf16().collect();

    unsafe {
        let mut hkey = HKEY::default();
        let res = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_path.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        );
        if res.is_err() {
            // Also try HKCU
            let res2 = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_path.as_ptr()),
                Some(0),
                KEY_READ,
                &mut hkey,
            );
            if res2.is_err() { return None; }
        }

        let mut buf = vec![0u16; 512];
        let mut buf_size = (buf.len() * 2) as u32;
        let mut reg_type = REG_VALUE_TYPE::default();

        let res = RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_size),
        );
        let _ = RegCloseKey(hkey);

        if res.is_err() { return None; }

        // Strip null terminator and convert
        let len = (buf_size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}
