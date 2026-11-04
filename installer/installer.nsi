; FFmpeg Encoder Windows installer (NSIS 3, cross-built on Linux with makensis).
; All identity values come from scripts/build-release.sh (-D...), so the same script can build the
; stable edition (master) and the experimental one side by side (different APP_ID / folders).
;
; - Per-user install by default (no administrator rights): %LOCALAPPDATA%\Programs\<APP_DIRNAME>
; - Optional "for everyone" (Program Files; asks for administrator rights)
; - Start Menu shortcut (default on), desktop shortcut (optional)
; - Listed in Add/Remove Programs (HKCU or HKLM ...\Uninstall\<APP_ID>)
; - Uninstall keeps settings/presets unless "Remove my settings and presets" is ticked
; - FFmpeg is not bundled (the app downloads it on request)
; Silent: setup.exe /S [/ALLUSERS] [/DESKTOP] [/D=path]   uninstall.exe /S [/REMOVESETTINGS]

Unicode true
; 64-bit installer (the app is x64 only; also runs on 64-bit-only Wine prefixes)
Target amd64-unicode
!ifndef APP_NAME
  !error "Build with scripts/build-release.sh"
!endif

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "nsDialogs.nsh"
!include "x64.nsh"

Name "${APP_NAME}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "${APP_NAME} ${APP_VERSION}"
ShowInstDetails show
ShowUninstDetails show

VIProductVersion "${VERSION4}"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "FileDescription" "${APP_NAME} setup"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" " "

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"

Var AllUsers        ; 1 = for everyone (HKLM, Program Files)
Var RadioMe
Var RadioAll
Var WantDesktop     ; 1 = desktop shortcut (silent /DESKTOP)

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_WELCOMEPAGE_TEXT "This installs ${APP_NAME} ${APP_VERSION}.$\r$\n$\r$\nIt keeps its own settings and presets and does not change any other version of FFmpeg Encoder.$\r$\n$\r$\nFFmpeg itself is not included: the app can download it for you (Help › Check for FFmpeg updates), or use a folder you already have."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP_NAME} now"

!insertmacro MUI_PAGE_WELCOME
Page custom InstallModePage InstallModeLeave
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_COMPONENTS
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ------------------------------------------------------------------ helpers
!macro SetMode
  ${If} $AllUsers == 1
    SetShellVarContext all
  ${Else}
    SetShellVarContext current
  ${EndIf}
!macroend

Function IsAdmin
  ; returns "1" on the stack when running with administrator rights
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 == "Admin"
    Push 1
  ${Else}
    Push 0
  ${EndIf}
FunctionEnd

Function DefaultDir
  ${If} $AllUsers == 1
    ${If} ${RunningX64}
      StrCpy $INSTDIR "$PROGRAMFILES64\${APP_DIRNAME}"
    ${Else}
      StrCpy $INSTDIR "$PROGRAMFILES\${APP_DIRNAME}"
    ${EndIf}
  ${Else}
    StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${APP_DIRNAME}"
  ${EndIf}
FunctionEnd

Function InstallModePage
  ${If} $AllUsers == 1
    Abort ; already elevated for everyone
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "Who should get ${APP_NAME}?" "Choose whether to install it just for you or for everyone on this PC."
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateRadioButton} 0 10u 100% 12u "&Just for me (recommended – no administrator rights needed)"
  Pop $RadioMe
  ${NSD_CreateLabel} 12u 24u 95% 20u "Installs to $LOCALAPPDATA\Programs\${APP_DIRNAME}."
  Pop $0
  ${NSD_CreateRadioButton} 0 50u 100% 12u "For &everyone on this PC (needs administrator rights)"
  Pop $RadioAll
  ${NSD_CreateLabel} 12u 64u 95% 20u "Installs to Program Files. Windows will ask for permission."
  Pop $0
  ${NSD_Check} $RadioMe
  nsDialogs::Show
FunctionEnd

Function InstallModeLeave
  ${NSD_GetState} $RadioAll $0
  ${If} $0 == ${BST_CHECKED}
    Call IsAdmin
    Pop $1
    ${If} $1 != 1
      ; restart the setup elevated, in "for everyone" mode
      ExecShell "runas" "$EXEPATH" "/ALLUSERS"
      Quit
    ${EndIf}
    StrCpy $AllUsers 1
  ${Else}
    StrCpy $AllUsers 0
  ${EndIf}
  Call DefaultDir
FunctionEnd

; ------------------------------------------------------------------ sections
Section "${APP_NAME} (required)" SecApp
  SectionIn RO
  !insertmacro SetMode
  SetOutPath "$INSTDIR"
  File "/oname=${APP_EXE}" "${EXE_PATH}"
  File "/oname=README.md" "${SRC_DIR}\README.md"
  File "/oname=NOTES.md" "${SRC_DIR}\NOTES.md"
  ; tells the app it is installed: settings go to %APPDATA%, not next to the exe
  FileOpen $0 "$INSTDIR\${MARKER}" w
  FileWrite $0 "Installed by setup. Settings and presets are kept in %APPDATA%\${DATA_DIR}.$\r$\n"
  FileClose $0
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ${If} $AllUsers == 1
    StrCpy $1 "HKLM"
    StrCpy $2 "/ALLUSERS"
  ${Else}
    StrCpy $1 "HKCU"
    StrCpy $2 "/CURRENTUSER"
  ${EndIf}
  ${GetSize} "$INSTDIR" "/S=0K" $3 $4 $5
  IntFmt $3 "0x%08X" $3
  ${If} $AllUsers == 1
    WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
    WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe" $2'
    WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" $2 /S'
    WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
    WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
    WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" $3
  ${Else}
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
    WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe" $2'
    WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" $2 /S'
    WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
    WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $3
  ${EndIf}
SectionEnd

Section "Start Menu shortcut" SecStart
  !insertmacro SetMode
  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
SectionEnd

Section /o "Desktop shortcut" SecDesktop
  !insertmacro SetMode
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
SectionEnd

Function .onInit
  StrCpy $AllUsers 0
  StrCpy $WantDesktop 0
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/ALLUSERS" $R1
  ${IfNot} ${Errors}
    StrCpy $AllUsers 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/DESKTOP" $R1
  ${IfNot} ${Errors}
    StrCpy $WantDesktop 1
  ${EndIf}
  ${If} $AllUsers == 1
    Call IsAdmin
    Pop $0
    ${If} $0 != 1
      ${If} ${Silent}
        SetErrorLevel 740 ; elevation required
        Quit
      ${EndIf}
      ExecShell "runas" "$EXEPATH" "/ALLUSERS"
      Quit
    ${EndIf}
  ${EndIf}
  ; /D=... on the command line sets $INSTDIR before .onInit
  ${If} $INSTDIR == ""
    Call DefaultDir
  ${EndIf}
  ${If} $WantDesktop == 1
    SectionGetFlags ${SecDesktop} $0
    IntOp $0 $0 | ${SF_SELECTED}
    SectionSetFlags ${SecDesktop} $0
  ${EndIf}
FunctionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "The program (FFmpeg is not included)."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecStart} "A shortcut in the Start Menu."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "A shortcut on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; ------------------------------------------------------------------ uninstaller
Function un.IsAdmin
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 == "Admin"
    Push 1
  ${Else}
    Push 0
  ${EndIf}
FunctionEnd

Section "un.${APP_NAME}" UnSecApp
  SectionIn RO
  !insertmacro SetMode
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\NOTES.md"
  Delete "$INSTDIR\${MARKER}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR" ; only if empty: files you put there (e.g. splash.png) stay
  ${If} $AllUsers == 1
    DeleteRegKey HKLM "${UNINST_KEY}"
  ${Else}
    DeleteRegKey HKCU "${UNINST_KEY}"
  ${EndIf}
SectionEnd

Section /o "un.Remove my settings and presets" UnSecSettings
  ; this user's settings, presets, logs and the FFmpeg this edition downloaded – never another edition's
  SetShellVarContext current
  RMDir /r "$APPDATA\${DATA_DIR}"
  RMDir /r "$LOCALAPPDATA\${DATA_DIR}"
SectionEnd

Function un.onInit
  StrCpy $AllUsers 0
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/ALLUSERS" $R1
  ${IfNot} ${Errors}
    StrCpy $AllUsers 1
    Call un.IsAdmin
    Pop $0
    ${If} $0 != 1
      ${If} ${Silent}
        SetErrorLevel 740
        Quit
      ${EndIf}
      ExecShell "runas" "$EXEPATH" "/ALLUSERS"
      Quit
    ${EndIf}
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/REMOVESETTINGS" $R1
  ${IfNot} ${Errors}
    SectionGetFlags ${UnSecSettings} $0
    IntOp $0 $0 | ${SF_SELECTED}
    SectionSetFlags ${UnSecSettings} $0
  ${EndIf}
FunctionEnd

!insertmacro MUI_UNFUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${UnSecApp} "Removes the program and its shortcuts."
  !insertmacro MUI_DESCRIPTION_TEXT ${UnSecSettings} "Also deletes your settings, presets, logs and the FFmpeg copy this version downloaded. Leave unticked to keep them."
!insertmacro MUI_UNFUNCTION_DESCRIPTION_END
