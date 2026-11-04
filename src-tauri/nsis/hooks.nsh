; Upgrade path for installs that shipped cadence.exe (Bolo 0.1.6 and earlier).
; The NSIS product name stays "Bolo.com", so the install folder
; ($LOCALAPPDATA\Bolo.com) and the uninstall registry key do not move.
; Only the exe name changes. Shortcuts are retargeted by the template once
; it sees the previous MainBinaryName in the uninstall key.

!macro NSIS_HOOK_PREINSTALL
  ; The updater closes ${MAINBINARYNAME}.exe. On 0.1.6 that process is still
  ; cadence.exe, so close it before the new file is copied and the old one deleted.
  ${If} ${FileExists} "$INSTDIR\cadence.exe"
    !insertmacro CheckIfAppIsRunning "$INSTDIR\cadence.exe" "${PRODUCTNAME}"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Belt and braces: delete the old exe even if the uninstall key never
  ; recorded MainBinaryName (the template also deletes the recorded name).
  Delete "$INSTDIR\cadence.exe"
  StrCpy $R6 "$INSTDIR\${MAINBINARYNAME}.exe"
  Push "${PRODUCTNAME}"
  Call BoloRewriteOneAutostart
  Push "Bolo"
  Call BoloRewriteOneAutostart
  Push "Cadence"
  Call BoloRewriteOneAutostart
  Push "cadence"
  Call BoloRewriteOneAutostart
!macroend

; $R6 is the new exe path, set by NSIS_HOOK_POSTINSTALL. The value name is on the stack.
; Rewrites a Run entry that still launches cadence.exe. Leaves every other program alone.
Function BoloRewriteOneAutostart
  Pop $R7
  ReadRegStr $R9 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "$R7"
  ${If} $R9 != ""
    StrCpy $R5 ""
    ${StrLoc} $R5 $R9 "cadence.exe" ">"
    ${If} $R5 == ""
      ${StrLoc} $R5 $R9 "Cadence.exe" ">"
    ${EndIf}
    ${If} $R5 != ""
      WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "$R7" "$\"$R6$\""
    ${EndIf}
  ${EndIf}
FunctionEnd
