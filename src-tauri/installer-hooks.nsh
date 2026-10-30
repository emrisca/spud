; The sign-in entry, written after Spud's own files are copied.
;
; Spud does not bundle or install virtual-cable drivers. They are VB-Audio's
; signed kernel drivers, the A+B pack is licensed per machine, and putting them
; in the build would be redistributing somebody else's driver. The README points
; at <https://vb-audio.com/Cable/> instead; the build never carries a driver.

!macro NSIS_HOOK_POSTINSTALL
  ; The sign-in entry, naming the copy that was just installed: a fresh install
  ; then comes back after a reboot without being asked for it. Spud's own
  ; Settings switch writes the same value, so the two agree.
  ;
  ; An entry that already names this executable is left exactly as it is, so
  ; running the installer again changes nothing. One that names a different path
  ; — an older install folder, or the build output somebody had it pointed at
  ; while trying the app — is rewritten, because otherwise it stays aimed at a
  ; binary that is about to disappear.
  ReadRegStr $R9 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Spud"
  StrCmp $R9 "" spud_signin_write 0
    ; Stored as `"C:\...\spud.exe" --tray`: drop the flag and compare the
    ; executable it names with the one just installed.
    StrCpy $R7 $R9 -8
    StrCmp $R7 '"$INSTDIR\spud.exe"' spud_signin_done
  spud_signin_write:
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Spud" '"$INSTDIR\spud.exe" --tray'
  spud_signin_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Left behind, this would have Windows try to launch a deleted executable at
  ; every sign-in.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Spud"
!macroend
