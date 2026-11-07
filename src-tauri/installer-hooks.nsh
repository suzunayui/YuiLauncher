; Moves people from the Electron launcher, which lived in another folder and updates
; itself with electron-updater. That updater runs this installer with --force-run.

!define ELECTRON_LAUNCHER_DIR "$LOCALAPPDATA\Programs\yui-launcher"
!define ELECTRON_LAUNCHER_UNINSTALLER "Uninstall YuiLauncher.exe"

; Runs first, so that removing the old shortcuts can't take the new ones with them.
!macro NSIS_HOOK_PREINSTALL
  IfFileExists "${ELECTRON_LAUNCHER_DIR}\${ELECTRON_LAUNCHER_UNINSTALLER}" 0 electron_launcher_gone
    Push $R8
    Push $R9
    StrCpy $R9 0
    electron_launcher_retry:
      IntOp $R9 $R9 + 1
      StrCpy $R8 "not started"
      ; _?= keeps the uninstaller from moving to a temp folder and returning at once.
      ExecWait '"${ELECTRON_LAUNCHER_DIR}\${ELECTRON_LAUNCHER_UNINSTALLER}" /S /currentuser _?=${ELECTRON_LAUNCHER_DIR}' $R8
      StrCmp $R8 "0" electron_launcher_removed
      ; When the app is open, the uninstaller closes it but stops there with an error.
      IntCmp $R9 5 electron_launcher_kept 0 electron_launcher_kept
      Sleep 2000
      Goto electron_launcher_retry
    electron_launcher_removed:
      ; It can't remove itself when run with _?=. Kept after a failure, to stay removable.
      Delete "${ELECTRON_LAUNCHER_DIR}\${ELECTRON_LAUNCHER_UNINSTALLER}"
      RMDir "${ELECTRON_LAUNCHER_DIR}"
    electron_launcher_kept:
    ClearErrors
    Pop $R9
    Pop $R8
  electron_launcher_gone:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Push $R9
  ClearErrors
  ${GetOptions} $CMDLINE "--force-run" $R9
  ${IfNot} ${Errors}
    Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
  ${EndIf}
  ClearErrors
  Pop $R9
!macroend
