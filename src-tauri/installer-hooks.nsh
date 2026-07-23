; Custom NSIS hooks for the vmux installer (see tauri.conf.json's
; bundle.windows.nsis.installerHooks). Adds $INSTDIR (where vmux.exe,
; vmuxctl.exe and vmuxd.exe all land — the latter two via `externalBin`)
; to the current user's PATH on install, and removes it again on uninstall,
; so `vmux`/`vmuxctl` work from any ordinary terminal, not just panes vmux
; itself spawned (which already get PATH augmented at runtime — see
; terminal/pty.rs's prepend_vmuxctl_dir).
;
; Deliberately uses only core NSIS instructions (StrCmp/StrLen/IntOp/StrCpy)
; instead of the StrFunc.nsh library — NSIS requires any function reachable
; from the uninstaller to only Call other "un."-prefixed functions, and
; StrFunc.nsh's macros need a specific un.-variant declaration to satisfy
; that; simpler and lower-risk to just not depend on it. This only handles
; the "our entry is missing entirely" and "our entry is exactly last"
; shapes (which is all that install/uninstall of *this* app alone ever
; produces) — if some other installer later reorders PATH so our entry
; ends up in the middle, uninstall leaves it alone rather than risk
; corrupting the rest of PATH; a stale, non-functional entry is harmless.

!include "WinMessages.nsh"

; Official Vim-for-Windows release used by the optional "install Vim too?"
; prompt below (see MaybeInstallVim) — vmux defaults to `vim` for opening
; files from the file tree, which silently does nothing useful if it's not
; on PATH. Bumped manually now and then; check
; https://github.com/vim/vim-win32-installer/releases for the current one.
!define VIM_INSTALLER_URL "https://github.com/vim/vim-win32-installer/releases/download/v9.2.0838/gvim_9.2.0838_x64_signed.exe"

!macro NSIS_HOOK_POSTINSTALL
  Call AddInstDirToPath
  Call MaybeInstallVim
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Call un.RemoveInstDirFromPath
!macroend

Function AddInstDirToPath
  ReadRegStr $0 HKCU "Environment" "Path"
  StrCmp $0 "" AddInstDirToPath_empty
  StrCmp $0 "$INSTDIR" AddInstDirToPath_done

  ; Already present as the trailing entry? Don't duplicate.
  StrLen $1 ";$INSTDIR"
  IntOp $1 0 - $1
  StrCpy $2 $0 "" $1
  StrCmp $2 ";$INSTDIR" AddInstDirToPath_done

  StrCpy $0 "$0;$INSTDIR"
  Goto AddInstDirToPath_write

  AddInstDirToPath_empty:
  StrCpy $0 "$INSTDIR"

  AddInstDirToPath_write:
  WriteRegExpandStr HKCU "Environment" "Path" "$0"
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  AddInstDirToPath_done:
FunctionEnd

; Optional: offer to install Vim (vmux's default file-tree "open file"
; command) via the official Windows installer, downloaded fresh from
; vim.org's GitHub releases rather than bundled — keeps our own installer
; small and always fetches whatever's current at install time. Uses
; NSISdl, which ships with NSIS itself (no extra plugin dependency, same
; reasoning as avoiding StrFunc.nsh above). `/S` for silent installation
; is NSIS's own convention (vim-win32-installer is itself NSIS-built) —
; not separately documented by the vim project, so worth confirming this
; still silent-installs cleanly next time VIM_INSTALLER_URL is bumped.
Function MaybeInstallVim
  MessageBox MB_YESNO|MB_ICONQUESTION \
    "Install Vim too? vmux uses it by default to open files from the file tree (change this anytime in Settings). This downloads the official installer from vim.org's GitHub releases (~10MB)." \
    IDNO MaybeInstallVim_done

  SetOutPath "$TEMP"
  NSISdl::download "${VIM_INSTALLER_URL}" "$TEMP\vmux_gvim_installer.exe"
  Pop $0
  StrCmp $0 "success" 0 MaybeInstallVim_failed
    ExecWait '"$TEMP\vmux_gvim_installer.exe" /S'
    Delete "$TEMP\vmux_gvim_installer.exe"
    Goto MaybeInstallVim_done

  MaybeInstallVim_failed:
    MessageBox MB_OK|MB_ICONEXCLAMATION \
      "Could not download Vim (no internet access right now, or vim.org's GitHub releases are unreachable). You can install it manually later from vim.org — vmux's Settings panel also lets you use a different editor command instead."

  MaybeInstallVim_done:
FunctionEnd

Function un.RemoveInstDirFromPath
  ReadRegStr $0 HKCU "Environment" "Path"

  StrCmp $0 "$INSTDIR" un.RemoveInstDirFromPath_clear un.RemoveInstDirFromPath_checksuffix

  un.RemoveInstDirFromPath_checksuffix:
  StrLen $1 ";$INSTDIR"
  IntOp $1 0 - $1
  StrCpy $2 $0 "" $1
  StrCmp $2 ";$INSTDIR" 0 un.RemoveInstDirFromPath_write
    StrCpy $0 $0 $1
    Goto un.RemoveInstDirFromPath_write

  un.RemoveInstDirFromPath_clear:
  StrCpy $0 ""

  un.RemoveInstDirFromPath_write:
  WriteRegExpandStr HKCU "Environment" "Path" "$0"
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
FunctionEnd
