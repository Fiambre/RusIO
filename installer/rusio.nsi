; RusIO installer (per-user, no administrator rights needed).
; Build: makensis /DVERSION=0.1.0 installer\rusio.nsi   (output goes to dist\)
Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif

Name "RusIO"
OutFile "..\dist\RusIO-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\RusIO"
InstallDirRegKey HKCU "Software\RusIO" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "..\crates\rusio-app\assets\icon.ico"
!define MUI_UNICON "..\crates\rusio-app\assets\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\rusio.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir RusIO"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Spanish"

!define UNKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\RusIO"

; An update is started by RusIO itself and runs while the app is still closing: wait until the
; old executable can be replaced (a running .exe cannot be deleted), up to ~15 seconds.
; A running .exe can be renamed even when it cannot be overwritten, so move it aside; the old
; process keeps running from the renamed file until it exits.
Function WaitForApp
  Delete "$INSTDIR\rusio.exe.old"
  StrCpy $0 0
  loop:
    ClearErrors
    IfFileExists "$INSTDIR\rusio.exe" 0 done
    Delete "$INSTDIR\rusio.exe"
    IfFileExists "$INSTDIR\rusio.exe" 0 done
    Rename "$INSTDIR\rusio.exe" "$INSTDIR\rusio.exe.old"
    IfFileExists "$INSTDIR\rusio.exe" 0 done
    IntOp $0 $0 + 1
    ; equal or more -> give up waiting; less -> keep trying
    IntCmp $0 30 done +1 done
    Sleep 500
    Goto loop
  done:
FunctionEnd

; After a silent (automatic) update, open the app again.
Function .onInstSuccess
  IfSilent 0 +2
  Exec '"$INSTDIR\rusio.exe"'
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  Call WaitForApp
  File "..\target\release\rusio.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortcut "$SMPROGRAMS\RusIO.lnk" "$INSTDIR\rusio.exe"

  WriteRegStr HKCU "Software\RusIO" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "DisplayName" "RusIO"
  WriteRegStr HKCU "${UNKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNKEY}" "Publisher" "Fiambre"
  WriteRegStr HKCU "${UNKEY}" "DisplayIcon" "$INSTDIR\rusio.exe"
  WriteRegStr HKCU "${UNKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNKEY}" "NoRepair" 1
SectionEnd

; Connections (connections.json) and saved passwords are left untouched on purpose.
Section "Uninstall"
  Delete "$INSTDIR\rusio.exe"
  Delete "$INSTDIR\rusio.exe.old"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\RusIO.lnk"
  DeleteRegKey HKCU "${UNKEY}"
  DeleteRegKey HKCU "Software\RusIO"
SectionEnd
