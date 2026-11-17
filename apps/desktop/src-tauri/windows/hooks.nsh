; NSIS 安装钩子（tauri.conf.json → bundle.windows.nsis.installerHooks）
;
; imgtool.exe 是给 Agent 用的 MCP 服务。Agent 开着时它一直在运行，
; Windows 不允许覆盖正在运行的程序，安装时会报 "Error opening file for writing"。
; 正在运行的程序可以改名：先把旧文件挪开，再写入新版本。
; 正在运行的 MCP 服务不受影响，在 Agent 里重启 MCP 后用上新版本。
; 挪开的旧文件在下次安装或卸载时清理（仍被占用则继续保留）。

!macro IMGTOOL_MOVE_LOCKED_SIDECAR
  Push $R9
  ; 清理以前挪开、现在已不再被占用的旧文件
  Delete "$INSTDIR\imgtool.exe.old*"
  ${If} ${FileExists} "$INSTDIR\imgtool.exe"
    ClearErrors
    Delete "$INSTDIR\imgtool.exe"
    ${If} ${Errors}
      StrCpy $R9 0
      ${While} ${FileExists} "$INSTDIR\imgtool.exe.old$R9"
        IntOp $R9 $R9 + 1
      ${EndWhile}
      ClearErrors
      Rename "$INSTDIR\imgtool.exe" "$INSTDIR\imgtool.exe.old$R9"
      ${If} ${Errors}
        DetailPrint "imgtool.exe is in use and could not be moved"
      ${Else}
        DetailPrint "imgtool.exe is in use (MCP server), moved to imgtool.exe.old$R9; restart MCP in your agent to use the new version"
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ClearErrors
  Pop $R9
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro IMGTOOL_MOVE_LOCKED_SIDECAR
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro IMGTOOL_MOVE_LOCKED_SIDECAR
!macroend
