# -*- coding: utf-8 -*-
; Based on Dioxus CLI v0.7.10's NSIS_TEMPLATE (MIT licensed).
; https://github.com/DioxusLabs/dioxus/blob/v0.7.10/packages/cli/src/bundler/windows.rs
; Keep Dioxus's Handlebars metadata and staged_files contract when upgrading dx.
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "nsDialogs.nsh"

!define LOCKED_IN_PRODUCT "{{product_name}}"
!define LOCKED_IN_VERSION "{{version}}"
!define LOCKED_IN_BINARY "{{main_binary_name}}"
!define LOCKED_IN_UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{bundle_id}}"
!define LOCKED_IN_STARTUP_VALUE "LockedIn"
!define LOCKED_IN_INSTANCE_MUTEX "Local\LockedIn.Desktop.Instance"

Name "${LOCKED_IN_PRODUCT}"
OutFile "{{output_path}}"
Unicode true
RequestExecutionLevel user
SetOverwrite on
AllowSkipFiles off
InstallDir "$LOCALAPPDATA\Programs\${LOCKED_IN_PRODUCT}"

VIProductVersion "{{version}}.0"
VIAddVersionKey "ProductName" "${LOCKED_IN_PRODUCT}"
VIAddVersionKey "FileVersion" "${LOCKED_IN_VERSION}"
VIAddVersionKey "ProductVersion" "${LOCKED_IN_VERSION}"
VIAddVersionKey "FileDescription" "{{short_description}}"
VIAddVersionKey "CompanyName" "{{publisher}}"
{{#if copyright}}
VIAddVersionKey "LegalCopyright" "{{copyright}}"
{{/if}}

!define MUI_ABORTWARNING
{{#if installer_icon}}
!define MUI_ICON "{{installer_icon}}"
{{/if}}
{{#if header_image}}
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_BITMAP "{{header_image}}"
{{/if}}
{{#if sidebar_image}}
!define MUI_WELCOMEFINISHPAGE_BITMAP "{{sidebar_image}}"
{{/if}}

!include "{{installer_hooks}}"

{{#if license}}
!insertmacro MUI_PAGE_LICENSE "{{license}}"
{{/if}}
Page custom ExistingInstallNotice
!define MUI_PAGE_CUSTOMFUNCTION_PRE ExistingInstallDirectory
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
{{#each additional_languages}}
!insertmacro MUI_LANGUAGE "{{this}}"
{{/each}}

Section "Install"
    ; Re-read registration at the point of mutation, including in silent mode.
    Call AcquireInstanceGuard
    Call ResolveInstallDirectory
    ClearErrors
    InitPluginsDir
    !insertmacro RequireInstallSuccess "Could not create the temporary staging directory."

    ; Finish extraction before touching the installed payload.
    !insertmacro StagePayloadFile "{{main_binary_path}}" ""
    {{#each staged_files}}
    !insertmacro StagePayloadFile "{{this.source}}" "{{this.target_dir}}"
    {{/each}}

    ; Validate all write/delete destinations before any installed file changes.
    Push "$INSTDIR"
    Call AssertUnredirectedPath
    Push "$INSTDIR\assets"
    Call AssertPlainAssetTree
    !insertmacro ValidatePayloadFile "{{main_binary_path}}" ""
    !insertmacro ValidatePayloadFile "uninstall.exe" ""
    {{#each staged_files}}
    !insertmacro ValidatePayloadFile "{{this.source}}" "{{this.target_dir}}"
    {{/each}}

    ; Overwrite incoming top-level files; unrelated top-level files stay put.
    !insertmacro CopyPayloadFile "{{main_binary_path}}" ""
    {{#each staged_files}}
    {{#unless this.target_dir}}
    !insertmacro CopyPayloadFile "{{this.source}}" ""
    {{/unless}}
    {{/each}}

    ; assets is disposable application content, regardless of its old filenames.
    ClearErrors
    RMDir /r "$INSTDIR\assets"
    !insertmacro RequireInstallSuccess "Could not remove the previous assets. Close applications using this folder and retry installation."
    {{#each staged_files}}
    {{#if this.target_dir}}
    !insertmacro CopyPayloadFile "{{this.source}}" "{{this.target_dir}}"
    {{/if}}
    {{/each}}

    SetOutPath "$INSTDIR"
    ClearErrors
    WriteUninstaller "$INSTDIR\uninstall.exe"
    !insertmacro RequireInstallSuccess "Could not write the uninstaller."

    ClearErrors
    CreateDirectory "$SMPROGRAMS\{{start_menu_folder}}"
    CreateShortcut "$SMPROGRAMS\{{start_menu_folder}}\${LOCKED_IN_PRODUCT}.lnk" "$INSTDIR\${LOCKED_IN_BINARY}"
    CreateShortcut "$SMPROGRAMS\{{start_menu_folder}}\Uninstall ${LOCKED_IN_PRODUCT}.lnk" "$INSTDIR\uninstall.exe"
    CreateShortcut "$DESKTOP\${LOCKED_IN_PRODUCT}.lnk" "$INSTDIR\${LOCKED_IN_BINARY}"
    !insertmacro RequireInstallSuccess "Could not create the application shortcuts."

    ; Upgrade, reinstall and downgrade share the same replacement flow.
    ClearErrors
    WriteRegStr HKCU "${LOCKED_IN_UNINSTALL_KEY}" "DisplayName" "${LOCKED_IN_PRODUCT}"
    WriteRegStr HKCU "${LOCKED_IN_UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "${LOCKED_IN_UNINSTALL_KEY}" "Publisher" "{{publisher}}"
    WriteRegStr HKCU "${LOCKED_IN_UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    !insertmacro RequireInstallSuccess "Could not register the installation."
    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    ClearErrors
    WriteRegDWORD HKCU "${LOCKED_IN_UNINSTALL_KEY}" "EstimatedSize" $0
    WriteRegStr HKCU "${LOCKED_IN_UNINSTALL_KEY}" "DisplayVersion" "${LOCKED_IN_VERSION}"
    !insertmacro RequireInstallSuccess "Could not register the installed version."

    {{#if install_webview}}
    ; BEGIN DIOXUS WEBVIEW INSTALLATION
    {{webview_install_code}}
    ; END DIOXUS WEBVIEW INSTALLATION
    {{/if}}
    Call ReleaseInstanceGuard
SectionEnd

Section "Uninstall"
    RMDir /r "$INSTDIR"
    RMDir /r "$SMPROGRAMS\{{start_menu_folder}}"
    Delete "$DESKTOP\${LOCKED_IN_PRODUCT}.lnk"
    DeleteRegKey HKCU "${LOCKED_IN_UNINSTALL_KEY}"
SectionEnd
