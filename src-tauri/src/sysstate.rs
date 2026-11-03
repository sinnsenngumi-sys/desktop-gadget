//! 動作状態（§7）: 電源、ゲーム中、ロック・スクリーンセーバー
//!
//! - 電源: GetSystemPowerStatus（AC/バッテリー、バッテリー節約）
//! - ユーザーの状態: SHQueryUserNotificationState（全画面アプリ、D3D全画面、プレゼンテーション、ロック等）
//! - 枠なし全画面: 手前の窓がモニター全体を覆っているか（Chromiumと同じ判定）
//! - ゲーム扱いのアプリ: 手前の窓の実行ファイル名を、設定の一覧と比べる

use serde::Serialize;

#[derive(Serialize, Clone, Default)]
pub struct SystemState {
    /// バッテリー駆動中
    pub on_battery: bool,
    /// バッテリー節約（省電力モード）がオン
    pub battery_saver: bool,
    /// 電池残量（%）。不明・電池なしは None
    pub battery_percent: Option<u8>,
    /// SHQueryUserNotificationState の値（1〜7）。取れないときは 0
    pub quns: i32,
    /// 手前の窓がモニター全体を覆っている（枠なし全画面を含む）
    pub fullscreen_window: bool,
    /// 手前の窓の実行ファイル名（小文字。このアプリ自身のときは None）
    pub foreground_exe: Option<String>,
    /// 手前のアプリが「ゲーム扱い」の一覧にある
    pub listed_game: bool,
    /// ゲーム中（全画面・D3D全画面・プレゼンテーション・枠なし全画面・一覧のアプリ）
    pub game: bool,
    /// どの理由でゲーム中と判定したか（確認用）
    pub game_reason: Option<String>,
    /// ロック中またはスクリーンセーバー中
    pub locked: bool,
}

/// QUNSの値を、画面の確認用の短い言葉にする
pub fn quns_label(q: i32) -> &'static str {
    match q {
        1 => "ロック・スクリーンセーバー",
        2 => "全画面アプリ",
        3 => "Direct3Dの全画面",
        4 => "プレゼンテーションモード",
        5 => "通常",
        6 => "サインイン直後の静かな時間",
        7 => "ストアアプリ",
        _ => "不明",
    }
}

/// 判定のまとめ（OSに依存しない部分。テストできるように分けておく）
pub fn decide(quns: i32, fullscreen_window: bool, listed_game: bool) -> (bool, Option<String>, bool) {
    let locked = quns == 1;
    // ロック画面も全画面の窓なので、ロック中は「ゲーム中」に数えない（ロックは別の設定で扱う）
    let reason = if locked {
        None
    } else if listed_game {
        Some("ゲーム扱いのアプリ".to_string())
    } else if quns == 2 || quns == 3 || quns == 4 {
        Some(quns_label(quns).to_string())
    } else if fullscreen_window {
        Some("枠なしの全画面".to_string())
    } else {
        None
    };
    (reason.is_some(), reason, locked)
}

/// "C:\\Games\\Foo.exe" や "foo" を、比べやすい "foo.exe" にそろえる
pub fn normalize_exe(s: &str) -> String {
    let base = s.trim().rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
    if base.is_empty() || base.ends_with(".exe") {
        base
    } else {
        format!("{}.exe", base)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, GWL_EXSTYLE, GWL_STYLE, WS_DLGFRAME,
        WS_EX_TOOLWINDOW, WS_EX_WINDOWEDGE, WS_THICKFRAME,
    };

    fn class_name(h: HWND) -> String {
        let mut buf = [0u16; 128];
        let n = unsafe { GetClassNameW(h, &mut buf) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn exe_of_pid(pid: u32) -> Option<String> {
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) };
        unsafe {
            let _ = CloseHandle(h);
        }
        r.ok()?;
        Some(normalize_exe(&String::from_utf16_lossy(&buf[..len as usize])))
    }

    /// 手前の窓がモニター全体を覆っているか（Chromium の IsFullScreenWindowMode と同じ考え方）
    fn covers_monitor(h: HWND) -> bool {
        // デスクトップやタスクバーは除く
        let cls = class_name(h);
        if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return false;
        }
        let mut wr = RECT::default();
        if unsafe { GetWindowRect(h, &mut wr) }.is_err() {
            return false;
        }
        let mon = unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONULL) };
        if mon.is_invalid() {
            return false;
        }
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !unsafe { GetMonitorInfoW(mon, &mut mi) }.as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        // 窓がモニターと同じか、はみ出して覆っている
        let covers = wr.left <= m.left && wr.top <= m.top && wr.right >= m.right && wr.bottom >= m.bottom;
        if !covers {
            return false;
        }
        let style = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32;
        let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
        let framed = style & (WS_DLGFRAME.0 | WS_THICKFRAME.0) != 0;
        let toolish = ex & (WS_EX_WINDOWEDGE.0 | WS_EX_TOOLWINDOW.0) != 0;
        !(framed || toolish)
    }

    pub fn read(game_exes: &[String]) -> SystemState {
        let mut s = SystemState::default();

        let mut ps = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut ps) }.is_ok() {
            s.on_battery = ps.ACLineStatus == 0;
            s.battery_saver = ps.SystemStatusFlag == 1;
            let no_battery = ps.BatteryFlag & 128 != 0 || ps.BatteryFlag == 255;
            s.battery_percent = if no_battery || ps.BatteryLifePercent > 100 { None } else { Some(ps.BatteryLifePercent) };
        }

        s.quns = unsafe { SHQueryUserNotificationState() }.map(|q| q.0).unwrap_or(0);

        let fg = unsafe { GetForegroundWindow() };
        if !fg.is_invalid() {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
            if pid != 0 && pid != std::process::id() {
                s.fullscreen_window = covers_monitor(fg);
                s.foreground_exe = exe_of_pid(pid);
            }
        }

        if let Some(exe) = &s.foreground_exe {
            s.listed_game = game_exes.iter().any(|g| normalize_exe(g) == *exe);
        }
        let (game, reason, locked) = decide(s.quns, s.fullscreen_window, s.listed_game);
        s.game = game;
        s.game_reason = reason;
        s.locked = locked;
        s
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn read(_game_exes: &[String]) -> SystemState {
        SystemState::default()
    }
}

pub use imp::read;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_names() {
        assert_eq!(normalize_exe("C:\\Games\\Foo.EXE"), "foo.exe");
        assert_eq!(normalize_exe(" valorant "), "valorant.exe");
        assert_eq!(normalize_exe(""), "");
    }

    #[test]
    fn decisions() {
        assert_eq!(decide(5, false, false).0, false);
        assert_eq!(decide(3, false, false).0, true);
        assert_eq!(decide(5, true, false).1.as_deref(), Some("枠なしの全画面"));
        assert_eq!(decide(5, false, true).1.as_deref(), Some("ゲーム扱いのアプリ"));
        let (g, _, l) = decide(1, false, false);
        assert!(!g && l);
    }
}
