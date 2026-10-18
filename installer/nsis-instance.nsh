Var InstanceHandle
Var InstanceAlreadyExists

; Share the application's named mutex rather than guessing from window titles
; (the window may be hidden in the tray). Use pointer-sized System plug-in types.
Function AcquireInstanceGuard
    retry:
        System::Call 'kernel32::CreateMutexW(p 0, i 0, w "${LOCKED_IN_INSTANCE_MUTEX}") p .r0 ?e'
        Pop $InstanceAlreadyExists
        StrCpy $InstanceHandle $0
        ${If} $InstanceHandle == 0
            !insertmacro FailInstallation "Could not check whether LockedIn is running."
        ${EndIf}
        ${If} $InstanceAlreadyExists == 183 ; ERROR_ALREADY_EXISTS
            ; A probe must not keep the mutex alive after the app quits.
            Call ReleaseInstanceGuard
            IfSilent busy
            MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION \
                "LockedIn is running. Choose Quit from its tray menu, then click Retry." \
                IDRETRY retry
            SetErrorLevel 2
            Quit ; Cancellation happens before any payload writes.
        ${EndIf}
        ; Holding a newly created mutex prevents app restart and concurrent
        ; installers while the payload is being replaced.
        Return
    busy:
        SetErrorLevel 2
        DetailPrint "LockedIn is running. Quit the application before installing."
        Quit
FunctionEnd

Function ReleaseInstanceGuard
    ${If} $InstanceHandle != ""
    ${AndIf} $InstanceHandle != 0
        System::Call 'kernel32::CloseHandle(p $InstanceHandle)'
        StrCpy $InstanceHandle ""
    ${EndIf}
FunctionEnd
