Var RegisteredDirectory

!macro FailInstallation message
    Call ReleaseInstanceGuard
    SetErrorLevel 1
    Abort "${message}"
!macroend

!macro RequireInstallSuccess message
    ${If} ${Errors}
        !insertmacro FailInstallation "${message}"
    ${EndIf}
!macroend

!include "${__FILEDIR__}\nsis-instance.nsh"

!macro StagePayloadFile source directory
    ClearErrors
    SetOutPath "$PLUGINSDIR\payload\${directory}"
    !insertmacro RequireInstallSuccess "Could not prepare the temporary payload directory."
    File "${source}"
    !insertmacro RequireInstallSuccess "Could not extract a packaged file."
!macroend

!macro ValidatePayloadFile source directory
    ; Old asset filenames/types do not constrain the incoming layout: their
    ; entire tree is already checked for redirects and will be removed.
    StrCpy $0 "${directory}\" 7
    ${If} $0 != "assets\"
        ${GetFileName} "${source}" $0
        Push "$INSTDIR\${directory}\$0"
        Call AssertUnredirectedPath
        System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR\${directory}\$0") i .r1'
        ${If} $1 != -1
            IntOp $1 $1 & 0x10 ; FILE_ATTRIBUTE_DIRECTORY
            ${If} $1 != 0
                !insertmacro FailInstallation "A packaged-file destination is a directory."
            ${EndIf}
        ${EndIf}
    ${EndIf}
!macroend

!macro CopyPayloadFile source directory
    ${GetFileName} "${source}" $0
    ClearErrors
    CreateDirectory "$INSTDIR\${directory}"
    !insertmacro RequireInstallSuccess "Could not create a payload destination."
    ; CopyFileW reports errors without shell dialogs, including in /S mode.
    StrCpy $3 0
    ${Do}
        System::Call 'kernel32::CopyFileW(w "$PLUGINSDIR\payload\${directory}\$0", w "$INSTDIR\${directory}\$0", i 0) i .r1 ?e'
        Pop $2
        ${If} $1 != 0
            ${ExitDo}
        ${EndIf}
        ${If} $2 != 32 ; ERROR_SHARING_VIOLATION
        ${AndIf} $2 != 33 ; ERROR_LOCK_VIOLATION
            ${ExitDo}
        ${EndIf}
        IntOp $3 $3 + 1
        ${If} $3 >= 10
            ${ExitDo}
        ${EndIf}
        Sleep 100 ; Allow transient antivirus/indexer handles to close.
    ${Loop}
    ${If} $1 == 0
        !insertmacro FailInstallation "Could not install $INSTDIR\${directory}\$0 (Windows error $2). Rerun the installer to repair the installation."
    ${EndIf}
!macroend

; Validate the full ancestor chain, including missing-file repair paths. Refuse
; junctions/symlinks instead of copying or recursively deleting through them.
Function AssertUnredirectedPath
    Exch $0
    Push $1
    Push $2
    loop:
        System::Call 'kernel32::GetFileAttributesW(w r0) i .r1 ?e'
        Pop $2
        ${If} $1 == -1
            ${If} $2 != 2 ; ERROR_FILE_NOT_FOUND
            ${AndIf} $2 != 3 ; ERROR_PATH_NOT_FOUND
                !insertmacro FailInstallation "Could not inspect an installation destination."
            ${EndIf}
        ${Else}
            IntOp $1 $1 & 0x400 ; FILE_ATTRIBUTE_REPARSE_POINT
            ${If} $1 != 0
                !insertmacro FailInstallation "Installation destinations must not use junctions or symbolic links."
            ${EndIf}
        ${EndIf}
        ${GetParent} "$0" $1
        ${If} $1 != ""
        ${AndIf} $1 != $0
            StrCpy $0 $1
            Goto loop
        ${EndIf}
    Pop $2
    Pop $1
    Pop $0
FunctionEnd

; Check nested assets too: the whole tree will be removed, not just incoming
; filenames. Preserve registers so recursive calls cannot lose enumeration state.
Function AssertPlainAssetTree
    Exch $0
    Push $1
    Push $2
    Push $3
    Push $0
    Call AssertUnredirectedPath
    System::Call 'kernel32::GetFileAttributesW(w r0) i .r3'
    ${If} $3 != -1
        IntOp $3 $3 & 0x10
        ${If} $3 == 0
            !insertmacro FailInstallation "The assets destination must be a directory."
        ${EndIf}
        FindFirst $1 $2 "$0\*"
        loop:
            IfErrors done
            ${If} $2 != "."
            ${AndIf} $2 != ".."
                Push "$0\$2"
                Call AssertUnredirectedPath
                System::Call 'kernel32::GetFileAttributesW(w "$0\$2") i .r3'
                IntOp $3 $3 & 0x10
                ${If} $3 != 0
                    Push "$0\$2"
                    Call AssertPlainAssetTree
                ${EndIf}
            ${EndIf}
            FindNext $1 $2
            Goto loop
        done:
            FindClose $1
    ${EndIf}
    Pop $3
    Pop $2
    Pop $1
    Pop $0
FunctionEnd

Function ResolveInstallDirectory
    ReadRegStr $RegisteredDirectory HKCU "${LOCKED_IN_UNINSTALL_KEY}" "InstallLocation"
    ${If} $RegisteredDirectory != ""
        ; NSIS validates fresh directory selections; registered repair paths
        ; must also be absolute and must not be a drive root.
        ${GetRoot} "$RegisteredDirectory" $0
        ${If} $0 == ""
            !insertmacro FailInstallation "The registered installation directory is invalid."
        ${EndIf}
        StrLen $1 $0
        StrCpy $1 $RegisteredDirectory 1 $1
        ${If} $1 != "\"
            !insertmacro FailInstallation "The registered installation directory must be absolute."
        ${EndIf}
        ClearErrors
        GetFullPathName $RegisteredDirectory "$RegisteredDirectory"
        !insertmacro RequireInstallSuccess "The registered installation directory is invalid."
        ${If} $RegisteredDirectory == "$0\"
        ${OrIf} $RegisteredDirectory == $0
            !insertmacro FailInstallation "The registered installation directory is invalid."
        ${EndIf}
        StrCpy $INSTDIR $RegisteredDirectory
    ${EndIf}
FunctionEnd

Function .onInit
    SetShellVarContext current
    Call ResolveInstallDirectory
FunctionEnd

Function ExistingInstallNotice
    ${If} $RegisteredDirectory == ""
        Abort ; Fresh installs go straight to the normal folder picker.
    ${EndIf}

    nsDialogs::Create 1018
    Pop $0
    ${If} $0 == error
        !insertmacro FailInstallation "Could not display the existing installation folder."
    ${EndIf}
    !insertmacro MUI_HEADER_TEXT "Existing installation found" "Review the installation folder before continuing."

    ${NSD_CreateLabel} 0 0 100% 16u "LockedIn is installed at:"
    Pop $0
    ; Wrap long paths and allow selection/copying, but never editing.
    nsDialogs::CreateControl EDIT ${WS_VISIBLE}|${WS_CHILD}|${WS_TABSTOP}|${ES_MULTILINE}|${ES_AUTOVSCROLL}|${ES_READONLY}|${WS_VSCROLL} ${WS_EX_CLIENTEDGE} 0 20u 100% 36u "$RegisteredDirectory"
    Pop $0
    ${NSD_CreateLabel} 0 68u 100% 24u "This installer will update that installation in place."
    Pop $0
    ${NSD_CreateLabel} 0 100u 100% 36u "To choose another location, uninstall LockedIn first. Your application settings will be preserved."
    Pop $0
    GetDlgItem $0 $HWNDPARENT 1
    ${NSD_SetText} $0 "$(^InstallBtn)"
    nsDialogs::Show
FunctionEnd

Function ExistingInstallDirectory
    ${If} $RegisteredDirectory != ""
        Abort ; Skip directory selection for an in-place update or repair.
    ${EndIf}
FunctionEnd

Function .onInstFailed
    Call ReleaseInstanceGuard
    SetErrorLevel 1
FunctionEnd

Function .onGUIEnd
    Call ReleaseInstanceGuard
FunctionEnd

Function un.onUninstSuccess
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${LOCKED_IN_STARTUP_VALUE}"
FunctionEnd
