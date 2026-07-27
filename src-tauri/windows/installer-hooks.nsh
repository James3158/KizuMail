!macro NSIS_HOOK_PREUNINSTALL
  ; Remove account secrets and local mail data before the installed binary is deleted.
  IfFileExists "$INSTDIR\kizumail.exe" 0 +2
    ExecWait '"$INSTDIR\kizumail.exe" --purge-user-data'
!macroend
