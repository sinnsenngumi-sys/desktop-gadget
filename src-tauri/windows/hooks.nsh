; DesktopGadget のインストーラーの追加処理（Tauri の NSIS フック）

; アンインストールしたあと、ログイン時の自動起動の登録も消す
; （残すと、削除した実行ファイルを起動しようとして失敗し続けるため）
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "DesktopGadget"
!macroend
