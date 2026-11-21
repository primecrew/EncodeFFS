; FFmpeg Encoder Windows installer (NSIS 3, cross-built on Linux with makensis).
; All identity values come from scripts/build-release.sh (-D...), so the same script can build the
; stable edition (master) and the experimental one side by side (different APP_ID / folders).
;
; - Per-user install by default (no administrator rights): %LOCALAPPDATA%\Programs\<APP_DIRNAME>
;   (fresh installs only; an upgrade keeps the existing InstallLocation, e.g. an older "FFmpeg Encoder (Codecs)" folder)
; - Optional "for everyone" (Program Files; asks for administrator rights)
; - Start Menu shortcut under <PUBLISHER>\<APP_NAME> (default on), desktop shortcut (optional)
; - On upgrade, leftover Start Menu / desktop shortcuts from earlier names are removed
; - Listed in Add/Remove Programs (HKCU or HKLM ...\Uninstall\<APP_ID>)
; - A newer setup detects that entry and upgrades in place (same folder and install mode).
;   The first page and the Ready to Install page show a banner
;   "Updating existing installation: <APP_NAME> v<old> → v<new>" and a read-only
;   "Installed in:" path (v(unknown) if DisplayVersion is missing). The who/where
;   pages are skipped and INSTDIR is forced to that folder (including when /D= is
;   passed). A fresh install is titled "New installation" and has the usual
;   directory page, labelled "Install to:". The last page says "Updated to v<new>
;   in <path>" or "Installed v<new> to <path>". Same version or an older setup can
;   reinstall. If the app is running, setup asks you to close it first. Settings,
;   presets and leftover portable files are not removed.
; - Uninstall keeps settings/presets unless "Remove my settings and presets" is ticked
; - FFmpeg is not in the installer (the app can use an existing install or download a copy)
; Silent: setup.exe /S [/ALLUSERS] [/DESKTOP] [/D=path]   uninstall.exe /S [/REMOVESETTINGS]

Unicode true
; 64-bit installer (the app is x64 only; also runs on 64-bit-only Wine prefixes)
Target amd64-unicode
!ifndef APP_NAME
  !error "Build with scripts/build-release.sh"
!endif
!ifndef OLD_APP_EXE
  !error "Build with scripts/build-release.sh (OLD_APP_EXE is required)"
!endif
!ifndef PREV_APP_NAME
  !define PREV_APP_NAME "FFmpeg Encoder"
!endif
!ifndef PREV_BRAND_APP_NAME
  !define PREV_BRAND_APP_NAME "FFS Just Encode"
!endif
!ifndef PUBLISHER
  !error "Build with scripts/build-release.sh (PUBLISHER is required)"
!endif
; Footer (BrandingText) and Start Menu group. Change this one define if the wording changes.
!ifndef BRAND_TEXT
  !define BRAND_TEXT "PrimeCrew Software"
!endif

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "WordFunc.nsh"
!include "nsDialogs.nsh"
!include "x64.nsh"

Name "${APP_NAME}"
OutFile "${OUTFILE}"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "${BRAND_TEXT}"
ShowInstDetails show
ShowUninstDetails show

VIProductVersion "${VERSION4}"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP_NAME} setup"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (C) 2026 ${PUBLISHER}"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"

Var AllUsers        ; 1 = for everyone (HKLM, Program Files)
Var RadioMe
Var RadioAll
Var WantDesktop     ; 1 = desktop shortcut (silent /DESKTOP)
Var Upgrade         ; 1 = existing install found (upgrade / reinstall / downgrade)
Var InstalledVersion
Var InstalledVerLabel  ; installed version, or "(unknown)"
Var UpgradeBanner      ; "Updating existing installation: … v<old> → v<new>"
Var FinishMsg

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_WELCOMEPAGE_TEXT "This installs ${APP_NAME} ${APP_VERSION}.$\r$\n$\r$\nIt keeps its own settings and presets and does not change any other edition of the app.$\r$\n$\r$\n${APP_NAME} can use your existing FFmpeg install, or download a copy for you (Help › Check for FFmpeg updates…)."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP_NAME} now"

!define MUI_WELCOMEPAGE_TITLE "New installation"
!define MUI_WELCOMEPAGE_TITLE_3LINES
!define MUI_PAGE_CUSTOMFUNCTION_SHOW WelcomeShow
!insertmacro MUI_PAGE_WELCOME
Page custom InstallModePage InstallModeLeave
!define MUI_DIRECTORYPAGE_TEXT_DESTINATION "Install to:"
!define MUI_PAGE_CUSTOMFUNCTION_PRE DirectoryPagePre
!define MUI_PAGE_CUSTOMFUNCTION_SHOW DirectoryPageShow
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_COMPONENTS
Page custom ReadyPage
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_TITLE_3LINES
!define MUI_PAGE_CUSTOMFUNCTION_SHOW FinishShow
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_COMPONENTS
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ------------------------------------------------------------------ helpers

Function TrimDir
  ; In/out: $0 — drop a trailing backslash (so $0\${APP_EXE} is a valid path)
  Push $1
  Push $2
  StrLen $1 $0
  ${If} $1 > 0
    IntOp $1 $1 - 1
    StrCpy $2 $0 1 $1
    ${If} $2 == "\"
      StrCpy $0 $0 $1
    ${EndIf}
  ${EndIf}
  Pop $2
  Pop $1
FunctionEnd

Function DetectExistingInstall
  ; When no /D= was given: HKCU first (unless /ALLUSERS was already requested), then HKLM.
  ; The folder must still contain ${APP_EXE} or leftover ${OLD_APP_EXE}. Sets $INSTDIR, $AllUsers, $InstalledVersion, $Upgrade.
  ${If} $AllUsers != 1
    ClearErrors
    ReadRegStr $0 HKCU "${UNINST_KEY}" "InstallLocation"
    ${If} $0 != ""
      Call TrimDir
      ${If} ${FileExists} "$0\${APP_EXE}"
      ${OrIf} ${FileExists} "$0\${OLD_APP_EXE}"
        ClearErrors
        ReadRegStr $1 HKCU "${UNINST_KEY}" "DisplayVersion"
        StrCpy $INSTDIR $0
        StrCpy $InstalledVersion $1
        StrCpy $AllUsers 0
        StrCpy $Upgrade 1
        Return
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ClearErrors
  ReadRegStr $0 HKLM "${UNINST_KEY}" "InstallLocation"
  ${If} $0 != ""
    Call TrimDir
    ${If} ${FileExists} "$0\${APP_EXE}"
    ${OrIf} ${FileExists} "$0\${OLD_APP_EXE}"
      ClearErrors
      ReadRegStr $1 HKLM "${UNINST_KEY}" "DisplayVersion"
      StrCpy $INSTDIR $0
      StrCpy $InstalledVersion $1
      StrCpy $AllUsers 1
      StrCpy $Upgrade 1
      Return
    ${EndIf}
  ${EndIf}
FunctionEnd

Function SplitVer
  ; In: $R0 = version; Out: $R1 = numeric prefix, $R2 = suffix after the first '-' (or empty)
  Push $R3
  Push $R4
  StrCpy $R1 ""
  StrCpy $R2 ""
  StrCpy $R3 0
  SplitVer_Loop:
    StrCpy $R4 $R0 1 $R3
    ${If} $R4 == ""
      StrCpy $R1 $R0
      Goto SplitVer_Done
    ${EndIf}
    ${If} $R4 == "-"
      StrCpy $R1 $R0 $R3
      IntOp $R3 $R3 + 1
      StrCpy $R2 $R0 "" $R3
      Goto SplitVer_Done
    ${EndIf}
    IntOp $R3 $R3 + 1
    Goto SplitVer_Loop
  SplitVer_Done:
  Pop $R4
  Pop $R3
FunctionEnd

Function CompareAppVersions
  ; In: $R5 = installed version, $R6 = this setup's version
  ; Out: $R7 = 0 same, 1 this setup is newer (upgrade), 2 this setup is older (downgrade)
  ${If} $R5 == $R6
    StrCpy $R7 0
    Return
  ${EndIf}
  Push $R0
  Push $R1
  Push $R2
  Push $R3
  Push $R4
  StrCpy $R0 $R5
  Call SplitVer
  StrCpy $R3 $R1
  StrCpy $R4 $R2
  StrCpy $R0 $R6
  Call SplitVer
  ${VersionCompare} "$R3" "$R1" $R7
  ${If} $R7 == 0
    ${If} $R4 == $R2
      StrCpy $R7 0
    ${ElseIf} $R4 == ""
      ; installed is a release of the same numbers; this setup is a prerelease
      StrCpy $R7 2
    ${ElseIf} $R2 == ""
      StrCpy $R7 1
    ${Else}
      ${VersionCompare} "$R4" "$R2" $R7
      ${If} $R7 == 1
        StrCpy $R7 2
      ${ElseIf} $R7 == 2
        StrCpy $R7 1
      ${EndIf}
    ${EndIf}
  ${EndIf}
  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
FunctionEnd

Function SetUpgradeTexts
  ${If} $InstalledVersion == ""
    StrCpy $InstalledVerLabel "(unknown)"
  ${Else}
    StrCpy $InstalledVerLabel "$InstalledVersion"
  ${EndIf}
  StrCpy $UpgradeBanner "Updating existing installation: ${APP_NAME} v$InstalledVerLabel → v${APP_VERSION}"
FunctionEnd

Function WelcomeShow
  ; Welcome title/text are a 195u column beside the wizard bitmap; long banner
  ; and path wrap. The Ready to Install page repeats them at full inner width.
  ; Segoe UI (falls back to the default UI font) includes the → glyph; Tahoma often does not.
  CreateFont $R0 "Segoe UI" 12 700
  CreateFont $R1 "Segoe UI" 10 400
  SendMessage $mui.WelcomePage.Title ${WM_SETFONT} $R0 0
  SendMessage $mui.WelcomePage.Text ${WM_SETFONT} $R1 0
  ${If} $Upgrade == 1
    ${NSD_SetText} $mui.WelcomePage.Title "$UpgradeBanner"
    ${NSD_SetText} $mui.WelcomePage.Text "Installed in:$\r$\n$INSTDIR$\r$\n$\r$\nThe install folder cannot be changed. Your settings and presets are kept.$\r$\n$\r$\n${APP_NAME} can use your existing FFmpeg install, or download a copy for you (Help › Check for FFmpeg updates…)."
  ${Else}
    ${NSD_SetText} $mui.WelcomePage.Title "New installation"
  ${EndIf}
FunctionEnd

Function DirectoryPagePre
  ${If} $Upgrade == 1
    Abort
  ${EndIf}
FunctionEnd

Function DirectoryPageShow
  !insertmacro MUI_HEADER_TEXT "New installation" "Choose the folder for ${APP_NAME}."
FunctionEnd

Function ReadyPage
  ${If} $Upgrade == 1
    !insertmacro MUI_HEADER_TEXT "Ready to Install" "The existing folder will be updated."
  ${Else}
    !insertmacro MUI_HEADER_TEXT "Ready to Install" "Click Install to begin."
  ${EndIf}
  nsDialogs::Create 1018
  Pop $0
  CreateFont $R0 "Segoe UI" 11 700
  CreateFont $R1 "Segoe UI" 10 400
  ${If} $Upgrade == 1
    ${NSD_CreateLabel} 0 0u 100% 28u "$UpgradeBanner"
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R0 1
    ${NSD_CreateLabel} 0 32u 100% 12u "Installed in:"
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R1 1
    ${NSD_CreateLabel} 0 46u 100% 28u "$INSTDIR"
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R1 1
    ${NSD_CreateLabel} 0 80u 100% 28u "The install folder cannot be changed. Your settings and presets are kept. Click Install to continue."
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R1 1
  ${Else}
    ${NSD_CreateLabel} 0 0u 100% 16u "New installation"
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R0 1
    ${NSD_CreateLabel} 0 24u 100% 12u "Install to:"
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R1 1
    ${NSD_CreateLabel} 0 38u 100% 28u "$INSTDIR"
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R1 1
    ${NSD_CreateLabel} 0 72u 100% 16u "Click Install to continue."
    Pop $1
    SendMessage $1 ${WM_SETFONT} $R1 1
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function FinishShow
  ${If} $Upgrade == 1
    StrCpy $FinishMsg "Updated to v${APP_VERSION} in $INSTDIR"
  ${Else}
    StrCpy $FinishMsg "Installed v${APP_VERSION} to $INSTDIR"
  ${EndIf}
  CreateFont $R0 "Segoe UI" 12 700
  CreateFont $R1 "Segoe UI" 10 400
  SendMessage $mui.FinishPage.Title ${WM_SETFONT} $R0 0
  SendMessage $mui.FinishPage.Text ${WM_SETFONT} $R1 0
  ${NSD_SetText} $mui.FinishPage.Title "$FinishMsg"
  ${NSD_SetText} $mui.FinishPage.Text "$FinishMsg"
FunctionEnd

Function PathIsLocked
  ; In: $R8 = path. Pushes 1 if the file exists and is locked, else 0.
  ${IfNot} ${FileExists} "$R8"
    Push 0
    Return
  ${EndIf}
  System::Call 'kernel32::CreateFileW(w "$R8", i 0x80000000, i 0, p 0, i 3, i 0, p 0) p .r0'
  StrCmp $0 "-1" PathIsLocked_Yes
  StrCmp $0 "0xFFFFFFFF" PathIsLocked_Yes
  StrCmp $0 "0xFFFFFFFFFFFFFFFF" PathIsLocked_Yes
  System::Call 'kernel32::CloseHandle(p r0)'
  Push 0
  Return
  PathIsLocked_Yes:
  Push 1
FunctionEnd

Function ExeIsInUse
  ; 1 if the installed exe (new or leftover old name) is locked or a window with this app's title is open.
  ${If} $InstalledVersion != ""
    FindWindow $0 "" "${APP_NAME} $InstalledVersion"
    ${If} $0 != 0
      Push 1
      Return
    ${EndIf}
  ${EndIf}
  FindWindow $0 "" "${APP_NAME} ${APP_VERSION}"
  ${If} $0 != 0
    Push 1
    Return
  ${EndIf}
  StrCpy $R8 "$INSTDIR\${APP_EXE}"
  Call PathIsLocked
  Pop $0
  ${If} $0 == 1
    Push 1
    Return
  ${EndIf}
  ${If} "${OLD_APP_EXE}" != "${APP_EXE}"
    StrCpy $R8 "$INSTDIR\${OLD_APP_EXE}"
    Call PathIsLocked
    Pop $0
    ${If} $0 == 1
      Push 1
      Return
    ${EndIf}
  ${EndIf}
  Push 0
FunctionEnd

Function CheckAppNotRunning
  CheckAppNotRunning_Retry:
  Call ExeIsInUse
  Pop $0
  ${If} $0 == 0
    Return
  ${EndIf}
  ${If} ${Silent}
    SetErrorLevel 1618
    Abort "${APP_NAME} is running. Close it and run setup again."
  ${EndIf}
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${APP_NAME} is running.$\r$\n$\r$\nPlease close it and click Retry to continue, or Cancel to stop setup." IDRETRY CheckAppNotRunning_Retry
  Abort
FunctionEnd

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
  ${If} $Upgrade == 1
  ${OrIf} $AllUsers == 1
    Abort ; upgrade in place, or already elevated for everyone
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
  Call CheckAppNotRunning
  SetOutPath "$INSTDIR"
  File "/oname=${APP_EXE}" "${EXE_PATH}"
  ; Upgrade from the pre-rename exe: leave the new file, remove the leftover name, and shortcuts are rewritten below.
  ${If} "${OLD_APP_EXE}" != "${APP_EXE}"
    Delete "$INSTDIR\${OLD_APP_EXE}"
  ${EndIf}
  File "/oname=README.md" "${SRC_DIR}\README.md"
  File "/oname=NOTES.md" "${SRC_DIR}\NOTES.md"
  File "/oname=LICENSE" "${SRC_DIR}\LICENSE"
  ; tells the app it is installed: settings go to %APPDATA%, not next to the exe
  FileOpen $0 "$INSTDIR\${MARKER}" w
  FileWrite $0 "Installed by setup. Settings and presets are kept in %APPDATA%\${DATA_PUBLISHER}\${DATA_DIR}.$\r$\n"
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
  ; Group this app with other PrimeCrew Software titles (Adobe-style brand folder)
  CreateDirectory "$SMPROGRAMS\${BRAND_TEXT}"
  CreateShortcut "$SMPROGRAMS\${BRAND_TEXT}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
  ; In-place upgrade: drop leftover shortcuts from earlier names so they are not duplicated.
  Delete "$SMPROGRAMS\${BRAND_TEXT}\${PREV_BRAND_APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${PREV_BRAND_APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${PREV_APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"
SectionEnd

Section /o "Desktop shortcut" SecDesktop
  !insertmacro SetMode
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
  Delete "$DESKTOP\${PREV_BRAND_APP_NAME}.lnk"
  Delete "$DESKTOP\${PREV_APP_NAME}.lnk"
SectionEnd

Function .onInit
  StrCpy $AllUsers 0
  StrCpy $WantDesktop 0
  StrCpy $Upgrade 0
  StrCpy $InstalledVersion ""
  StrCpy $InstalledVerLabel ""
  StrCpy $UpgradeBanner ""
  StrCpy $FinishMsg ""
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
  ; Elevate after DetectExistingInstall so an all-users upgrade can request rights.
  ; Always look up the existing install. On an upgrade, force INSTDIR to that
  ; folder even if /D= was passed. A fresh install still honours /D=.
  Call DetectExistingInstall
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
  ${If} $INSTDIR == ""
    Call DefaultDir
  ${EndIf}
  ${If} $Upgrade == 1
    Call SetUpgradeTexts
  ${EndIf}
  ${If} $WantDesktop == 1
    SectionGetFlags ${SecDesktop} $0
    IntOp $0 $0 | ${SF_SELECTED}
    SectionSetFlags ${SecDesktop} $0
  ${EndIf}
FunctionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "The program. You can use your existing FFmpeg, or download a copy in the app."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecStart} "A shortcut in the Start Menu, under ${BRAND_TEXT}."
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
  Delete "$SMPROGRAMS\${BRAND_TEXT}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${BRAND_TEXT}\${PREV_BRAND_APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${BRAND_TEXT}" ; only if empty: other PrimeCrew apps keep the folder
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${PREV_BRAND_APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${PREV_APP_NAME}.lnk"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  Delete "$DESKTOP\${PREV_BRAND_APP_NAME}.lnk"
  Delete "$DESKTOP\${PREV_APP_NAME}.lnk"
  Delete "$INSTDIR\${APP_EXE}"
  ${If} "${OLD_APP_EXE}" != "${APP_EXE}"
    Delete "$INSTDIR\${OLD_APP_EXE}"
  ${EndIf}
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\NOTES.md"
  Delete "$INSTDIR\LICENSE"
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
  RMDir /r "$APPDATA\${DATA_PUBLISHER}\${DATA_DIR}"
  RMDir "$APPDATA\${DATA_PUBLISHER}" ; only if empty: other PrimeCrew apps keep the folder
  RMDir /r "$LOCALAPPDATA\${DATA_PUBLISHER}\${DATA_DIR}"
  RMDir "$LOCALAPPDATA\${DATA_PUBLISHER}"
  RMDir /r "$APPDATA\${LEGACY_DATA_DIR}"
  RMDir /r "$LOCALAPPDATA\${LEGACY_DATA_DIR}"
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
