; Onsite hooks for Tauri's NSIS installer (tauri.conf.json > bundle > windows > nsis > installerHooks).
; The macros expand inside the template's `Section Uninstall`, so its variables ($UpdateMode,
; $DeleteAppDataCheckboxState) and includes (LogicLib, FileFunc) are in scope.
;
; A real uninstall (not /UPDATE, which an in-app update passes) always removes what Onsite
; downloaded or wrote as scratch: the voice models (~580 MB), the log and temp files.
; Interviews, settings, API keys and the WebView's storage (theme, last setup, resume copy) go
; when "Delete the application data" is ticked, or with /PURGE (Settings > Storage > Uninstall).

!macro OnsiteRemoveTemp PATTERN
  Push $0
  Push $1
  FindFirst $0 $1 "$TEMP\${PATTERN}"
  ${DoWhile} $1 != ""
    RMDir /r "$TEMP\$1"
    FindNext $0 $1
  ${Loop}
  FindClose $0
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Push $0
  ClearErrors
  ${GetOptions} $CMDLINE "/PURGE" $0
  ${IfNot} ${Errors}
    StrCpy $DeleteAppDataCheckboxState 1
  ${EndIf}
  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    SetShellVarContext current
    RMDir /r "$APPDATA\${BUNDLEID}\models"
    RMDir /r "$LOCALAPPDATA\Onsite"
    RMDir /r "$TEMP\onsite-update"
    RMDir /r "$TEMP\onsite-stt"
    !insertmacro OnsiteRemoveTemp "onsite-round-*"
    !insertmacro OnsiteRemoveTemp "onsite-job-*"
  ${EndIf}
!macroend
