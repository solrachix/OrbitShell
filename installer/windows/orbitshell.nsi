!include "MUI2.nsh"

!define APP_NAME "OrbitShell"
!define APP_EXE "orbitshell.exe"
!define APP_PUBLISHER "OrbitShell"
!ifndef APP_VERSION
!define APP_VERSION "0.1.0"
!endif
!ifndef OUTPUT_NAME
!define OUTPUT_NAME "OrbitShell-Setup-${APP_VERSION}.exe"
!endif

Name "${APP_NAME}"
OutFile "${OUTPUT_NAME}"
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
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\OrbitShell" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\OrbitShell" "Publisher" "${APP_PUBLISHER}"
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
