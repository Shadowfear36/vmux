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

!macro NSIS_HOOK_POSTINSTALL
  Call AddInstDirToPath
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
