!include "MUI2.nsh"

!define APP_NAME "OrbitShell"
!define APP_EXE "orbitshell.exe"
!define APP_PUBLISHER "OrbitShell"
!define APP_VERSION "0.1.0"

Name "${APP_NAME}"
OutFile "OrbitShell-Setup.exe"
Icon "..\\..\\assets\\logo.ico"
InstallDir "$PROGRAMFILES64\\OrbitShell"
InstallDirRegKey HKCU "Software\\OrbitShell" "InstallDir"
RequestExecutionLevel user

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"
  File "..\\..\\target\\release\\${APP_EXE}"
  File "..\\..\\orbitshell_rules.json"
  File "..\\..\\assets\\logo.ico"

  CreateDirectory "$SMPROGRAMS\\OrbitShell"
  CreateShortcut "$SMPROGRAMS\\OrbitShell\\OrbitShell.lnk" "$INSTDIR\\${APP_EXE}" "" "$INSTDIR\\logo.ico"
  CreateShortcut "$DESKTOP\\OrbitShell.lnk" "$INSTDIR\\${APP_EXE}" "" "$INSTDIR\\logo.ico"

  WriteUninstaller "$INSTDIR\\Uninstall.exe"
  WriteRegStr HKCU "Software\\OrbitShell" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\OrbitShell" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\OrbitShell" "UninstallString" "$INSTDIR\\Uninstall.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\\${APP_EXE}"
  Delete "$INSTDIR\\orbitshell_rules.json"
  Delete "$INSTDIR\\logo.ico"
  Delete "$INSTDIR\\Uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\\OrbitShell\\OrbitShell.lnk"
  RMDir "$SMPROGRAMS\\OrbitShell"
  Delete "$DESKTOP\\OrbitShell.lnk"
  DeleteRegKey HKCU "Software\\OrbitShell"
  DeleteRegKey HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\OrbitShell"
SectionEnd
