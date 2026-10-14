; A separate branded icon path avoids Explorer reusing a cached legacy exe icon.
!macro SONOJOT_REFRESH_ICON LINK
  !insertmacro IsShortcutTarget "${LINK}" "$INSTDIR\${MAINBINARYNAME}.exe"
  Pop $0
  ${If} $0 = 1
    CreateShortcut "${LINK}" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\sonojot.ico" 0
    !insertmacro SetLnkAppUserModelId "${LINK}"
  ${EndIf}
!macroend

; A renamed binary must still wait for the previous installed process to exit.
!macro NSIS_HOOK_PREINSTALL
  ${If} ${FileExists} "$INSTDIR\eavesdropper-next.exe"
    !insertmacro CheckIfAppIsRunning "eavesdropper-next.exe" "${PRODUCTNAME}"
  ${EndIf}
!macroend

; Remove an old display shortcut only when it points to this installation.
!macro SONOJOT_REMOVE_OLD_SHORTCUT LINK
  !insertmacro IsShortcutTarget "${LINK}" "$INSTDIR\eavesdropper-next.exe"
  Pop $0
  ${If} $0 = 1
    Delete "${LINK}"
  ${Else}
    !insertmacro IsShortcutTarget "${LINK}" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      Delete "${LINK}"
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Keep an older build's startup value pointing at the renamed binary. Only
  ; this installation's exact old command is replaced; another installation is
  ; untouched. At its next launch the app renames the value to "SonoJot" and
  ; carries Task Manager's enabled/disabled state with it (platform.rs).
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "EavesdropperNext"
  ${If} $0 == '$\"$INSTDIR\eavesdropper-next.exe$\" --startup'
  ${OrIf} $0 == '$\"$INSTDIR\sonojot.exe$\" --startup'
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "EavesdropperNext" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --startup'
  ${EndIf}
  !insertmacro SONOJOT_REMOVE_OLD_SHORTCUT "$SMPROGRAMS\Eavesdropper Next.lnk"
  !insertmacro SONOJOT_REMOVE_OLD_SHORTCUT "$DESKTOP\Eavesdropper Next.lnk"
  !insertmacro SONOJOT_REFRESH_ICON "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  !insertmacro SONOJOT_REFRESH_ICON "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  !insertmacro SONOJOT_REFRESH_ICON "$DESKTOP\${PRODUCTNAME}.lnk"
  ; Start search keeps a separate per-AppID PNG, unaffected by shortcut icon
  ; changes. Invalidate this app's entry only, at each existing display scale.
  Push $0
  Push $1
  FindFirst $0 $1 "$LOCALAPPDATA\Packages\Microsoft.Windows.Search_cw5n1h2txyewy\LocalState\AppIconCache\*"
  IfErrors sonojot_cache_done
  sonojot_cache_next:
    StrCmp $1 "" sonojot_cache_close
    StrCmp $1 "." sonojot_cache_advance
    StrCmp $1 ".." sonojot_cache_advance
    Delete "$LOCALAPPDATA\Packages\Microsoft.Windows.Search_cw5n1h2txyewy\LocalState\AppIconCache\$1\studio_codystevens_eavesdropper_next"
  sonojot_cache_advance:
    FindNext $0 $1
    Goto sonojot_cache_next
  sonojot_cache_close:
    FindClose $0
  sonojot_cache_done:
  Pop $1
  Pop $0
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
