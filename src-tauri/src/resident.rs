//! フェーズ5：常駐化（§6・§7）
//!
//! - デスクトップに付ける: 窓の持ち主をデスクトップ（Progman）にし、最背面・非アクティブにする。
//!   Win+D（デスクトップの表示）でも残し、マウス操作（ドラッグ、ボタン）は受け付ける。
//!   万一隠された・最小化されたときは、見張りのスレッドが表示し直す
//! - 位置の保存と復元、画面外に出たときの戻し（外付けモニターを外したときなど）
//! - 設定ファイル（settings.json）の読み書き、書き出し・読み込み
//! - 自動起動（HKCU\...\Run）
//! - トレイアイコンと、別窓の設定画面

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub struct Resident {
    /// デスクトップに付けるか（false は通常の最背面の窓）
    pub attach: AtomicBool,
    /// トレイから「隠す」を選んだ（このときは表示し直さない）
    pub user_hidden: AtomicBool,
    pub dir: PathBuf,
    moved: Mutex<Option<(i32, i32, Instant)>>,
}

impl Resident {
    pub fn new(dir: PathBuf) -> Self {
        Resident { attach: AtomicBool::new(true), user_hidden: AtomicBool::new(false), dir, moved: Mutex::new(None) }
    }
    pub fn note_move(&self, x: i32, y: i32) {
        if let Ok(mut m) = self.moved.lock() {
            *m = Some((x, y, Instant::now()));
        }
    }
    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

/// ファイルを、書きかけで壊れないように書く（一時ファイルに書いてから置き換える）
fn write_atomic(path: &std::path::Path, data: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("保存できませんでした（{}）: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("保存できませんでした（{}）: {}", path.display(), e))
}

/// 設定のJSONとして受け付けるか（{"v":{...}} の形で、大きすぎない）
pub fn valid_settings(json: &str) -> Result<(), String> {
    if json.len() > 512 * 1024 {
        return Err("設定が大きすぎます".into());
    }
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("設定ファイルの形式が正しくありません: {}", e))?;
    if !v.get("v").map(|x| x.is_object()).unwrap_or(false) {
        return Err("このアプリの設定ファイルではないようです（\"v\" がありません）".into());
    }
    Ok(())
}

// ---------------------------------------------------------------
// デスクトップに付ける（Windows）
// ---------------------------------------------------------------

#[cfg(windows)]
mod win {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn hwnd_from(raw: isize) -> HWND {
        HWND(raw as *mut core::ffi::c_void)
    }

    fn progman() -> Option<HWND> {
        unsafe { FindWindowW(w!("Progman"), PCWSTR::null()) }.ok().filter(|h| !h.is_invalid())
    }

    /// attach=true: 持ち主をデスクトップにし、非アクティブ・タスクバーなしにする。false: 元に戻す
    pub fn apply(h: HWND, attach: bool) -> Result<(), String> {
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            let want = if attach { ex | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 } else { (ex & !WS_EX_NOACTIVATE.0) | WS_EX_TOOLWINDOW.0 };
            SetWindowLongPtrW(h, GWL_EXSTYLE, want as isize);
            let owner = if attach {
                progman().ok_or("デスクトップの窓（Progman）が見つかりませんでした")?
            } else {
                HWND::default()
            };
            SetWindowLongPtrW(h, GWLP_HWNDPARENT, owner.0 as isize);
            SetWindowPos(h, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// 見張り: 隠された・最小化されたら表示し直す。スタイルが外れていたら付け直す
    pub fn guard(h: HWND, attach: bool) {
        unsafe {
            if IsIconic(h).as_bool() || !IsWindowVisible(h).as_bool() {
                let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            }
            if attach {
                let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
                let owner = GetWindowLongPtrW(h, GWLP_HWNDPARENT);
                if ex & WS_EX_NOACTIVATE.0 == 0 || owner == 0 {
                    let _ = apply(h, true);
                }
            }
        }
    }
}

fn main_hwnd(w: &WebviewWindow) -> Option<isize> {
    #[cfg(windows)]
    {
        w.hwnd().ok().map(|h| h.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        None
    }
}

pub fn apply_desktop_mode(app: &AppHandle, attach: bool) -> Result<(), String> {
    let st = app.state::<Resident>();
    st.attach.store(attach, Ordering::SeqCst);
    let w = app.get_webview_window("main").ok_or("ガジェットの窓が見つかりません")?;
    #[cfg(windows)]
    {
        let h = main_hwnd(&w).ok_or("窓のハンドルを取得できません")?;
        win::apply(win::hwnd_from(h), attach)?;
    }
    #[cfg(not(windows))]
    {
        let _ = w;
    }
    Ok(())
}

// ---------------------------------------------------------------
// 位置（保存・復元・画面外からの戻し）
// ---------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct SavedPos {
    x: i32,
    y: i32,
}

#[derive(Serialize, Clone)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub primary: bool,
    /// ガジェットがいまこのモニターにある
    pub current: bool,
}

pub fn monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    let Some(w) = app.get_webview_window("main") else { return Vec::new() };
    let list = w.available_monitors().unwrap_or_default();
    let prim = w.primary_monitor().ok().flatten();
    let cur = w.current_monitor().ok().flatten();
    let same = |a: &tauri::Monitor, b: &Option<tauri::Monitor>| b.as_ref().map(|b| b.position() == a.position() && b.size() == a.size()).unwrap_or(false);
    list.iter()
        .enumerate()
        .map(|(i, m)| MonitorInfo {
            index: i,
            name: m.name().cloned().unwrap_or_else(|| format!("モニター{}", i + 1)),
            x: m.position().x,
            y: m.position().y,
            width: m.size().width,
            height: m.size().height,
            scale: m.scale_factor(),
            primary: same(m, &prim),
            current: same(m, &cur),
        })
        .collect()
}

/// (x, y) がどれかのモニターの中にあるか
fn on_some_monitor(ms: &[MonitorInfo], x: i32, y: i32) -> bool {
    ms.iter().any(|m| x >= m.x && y >= m.y && x < m.x + m.width as i32 && y < m.y + m.height as i32)
}

/// モニターの左上から少し内側（論理40px）
fn corner(m: &MonitorInfo) -> (i32, i32) {
    (m.x + (40.0 * m.scale) as i32, m.y + (40.0 * m.scale) as i32)
}

pub fn move_main(app: &AppHandle, x: i32, y: i32) -> Result<(), String> {
    let w = app.get_webview_window("main").ok_or("ガジェットの窓が見つかりません")?;
    w.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
    app.state::<Resident>().note_move(x, y);
    Ok(())
}

pub fn reset_position(app: &AppHandle) -> Result<(), String> {
    let ms = monitors(app);
    let m = ms.iter().find(|m| m.primary).or(ms.first()).ok_or("モニターが見つかりません")?;
    let (x, y) = corner(m);
    move_main(app, x, y)
}

pub fn move_to_monitor(app: &AppHandle, index: usize) -> Result<(), String> {
    let ms = monitors(app);
    let m = ms.get(index).ok_or("そのモニターは見つかりません")?;
    let (x, y) = corner(m);
    move_main(app, x, y)
}

/// 起動時: 保存した位置に戻す（画面の外なら、メインのモニターの左上）
pub fn restore_position(app: &AppHandle) {
    let st = app.state::<Resident>();
    let saved: Option<SavedPos> = std::fs::read_to_string(st.file("window.json")).ok().and_then(|s| serde_json::from_str(&s).ok());
    let ms = monitors(app);
    match saved {
        Some(p) if on_some_monitor(&ms, p.x + 20, p.y + 20) => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_position(PhysicalPosition::new(p.x, p.y));
            }
        }
        Some(_) => {
            let _ = reset_position(app);
        }
        None => {}
    }
}

/// 見張りのスレッド: Win+Dなどで隠れたら戻す、画面外に出たら戻す、動かした位置を保存する
pub fn spawn_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let mut n: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            n += 1;
            let st = app.state::<Resident>();
            let Some(w) = app.get_webview_window("main") else { continue };

            #[cfg(windows)]
            if !st.user_hidden.load(Ordering::SeqCst) {
                if let Some(h) = main_hwnd(&w) {
                    win::guard(win::hwnd_from(h), st.attach.load(Ordering::SeqCst));
                }
            }

            // 2秒ごと: 窓の中心がどのモニターにもなければ、メインのモニターの左上へ
            if n.is_multiple_of(4) && !st.user_hidden.load(Ordering::SeqCst) {
                if let (Ok(p), Ok(s)) = (w.outer_position(), w.outer_size()) {
                    let ms = monitors(&app);
                    if !ms.is_empty() && !on_some_monitor(&ms, p.x + s.width as i32 / 2, p.y + (s.height as i32).min(80) / 2) {
                        let _ = reset_position(&app);
                    }
                }
            }

            // 動かし終わって1秒たったら、位置を保存
            let pending = st.moved.lock().ok().and_then(|m| *m);
            if let Some((x, y, t)) = pending {
                if t.elapsed() > Duration::from_secs(1) {
                    let _ = write_atomic(&st.file("window.json"), &serde_json::to_string(&SavedPos { x, y }).unwrap_or_default());
                    if let Ok(mut m) = st.moved.lock() {
                        if m.map(|(_, _, t2)| t2 == t).unwrap_or(false) {
                            *m = None;
                        }
                    }
                }
            }
        }
    });
}

// ---------------------------------------------------------------
// 設定ファイル
// ---------------------------------------------------------------

pub fn settings_read(app: &AppHandle) -> Option<String> {
    std::fs::read_to_string(app.state::<Resident>().file("settings.json")).ok().filter(|s| valid_settings(s).is_ok())
}

pub fn settings_write(app: &AppHandle, json: &str) -> Result<(), String> {
    valid_settings(json)?;
    write_atomic(&app.state::<Resident>().file("settings.json"), json)
}

pub fn settings_dir(app: &AppHandle) -> String {
    app.state::<Resident>().dir.display().to_string()
}

// ---------------------------------------------------------------
// 自動起動（ログイン時に起動）
// ---------------------------------------------------------------

#[derive(Serialize)]
pub struct AutoStart {
    pub on: bool,
    /// 登録されている実行ファイルが、いま動いているものと違う（開発版とリリース版など）
    pub other_exe: bool,
    pub exe: String,
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_NAME: &str = "DesktopGadget";

fn exe_cmd() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("\"{}\"", exe.display()))
}

#[cfg(windows)]
pub fn autostart_get() -> Result<AutoStart, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let cmd = exe_cmd()?;
    let cur: Option<String> = RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY).ok().and_then(|k| k.get_value(RUN_NAME).ok());
    Ok(AutoStart { on: cur.is_some(), other_exe: cur.as_ref().map(|c| !c.eq_ignore_ascii_case(&cmd)).unwrap_or(false), exe: cmd })
}

#[cfg(windows)]
pub fn autostart_set(on: bool) -> Result<AutoStart, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY).map_err(|e| e.to_string())?;
    if on {
        key.set_value(RUN_NAME, &exe_cmd()?).map_err(|e| format!("自動起動を登録できませんでした: {}", e))?;
    } else {
        match key.delete_value(RUN_NAME) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("自動起動を解除できませんでした: {}", e)),
        }
    }
    autostart_get()
}

#[cfg(not(windows))]
pub fn autostart_get() -> Result<AutoStart, String> {
    Err("Windows専用の機能です".into())
}
#[cfg(not(windows))]
pub fn autostart_set(_on: bool) -> Result<AutoStart, String> {
    Err("Windows専用の機能です".into())
}

// ---------------------------------------------------------------
// 設定画面（別窓）とトレイアイコン
// ---------------------------------------------------------------

pub fn open_settings(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("DesktopGadget の設定")
        .inner_size(460.0, 820.0)
        .min_inner_size(380.0, 400.0)
        .resizable(true)
        .center()
        .build()
        .map(|_| ())
        .map_err(|e| format!("設定画面を開けませんでした: {}", e))
}

/// 窓を作る処理は、イベントの処理中ではなく別のタスクで行う（Windowsで固まるのを避ける）
pub fn open_settings_later(app: &AppHandle) {
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = open_settings(&h);
    });
}

pub fn toggle_visible(app: &AppHandle) {
    let st = app.state::<Resident>();
    let Some(w) = app.get_webview_window("main") else { return };
    if w.is_visible().unwrap_or(true) {
        st.user_hidden.store(true, Ordering::SeqCst);
        let _ = w.hide();
    } else {
        st.user_hidden.store(false, Ordering::SeqCst);
        let _ = w.show();
        let _ = apply_desktop_mode(app, st.attach.load(Ordering::SeqCst));
    }
}

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let it = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &it("settings", "設定を開く")?,
            &PredefinedMenuItem::separator(app)?,
            &it("digital", "時計をデジタルにする")?,
            &it("analog", "時計をアナログにする")?,
            &PredefinedMenuItem::separator(app)?,
            &it("toggle", "表示する／隠す")?,
            &it("lock", "位置の固定を切り替える")?,
            &it("reset", "左上に戻す")?,
            &PredefinedMenuItem::separator(app)?,
            &it("quit", "終了")?,
        ],
    )?;

    let mut b = TrayIconBuilder::with_id("main")
        .tooltip("DesktopGadget")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "settings" => open_settings_later(app),
            "digital" | "analog" | "lock" => {
                let _ = app.emit_to("main", "tray", ev.id().as_ref().to_string());
            }
            "toggle" => toggle_visible(app),
            "reset" => {
                let _ = reset_position(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                open_settings_later(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_validation() {
        assert!(valid_settings(r#"{"v":{"sec":true},"r":{},"tc":{}}"#).is_ok());
        assert!(valid_settings(r#"{"x":1}"#).is_err());
        assert!(valid_settings("not json").is_err());
    }

    #[test]
    fn monitor_bounds() {
        let ms = vec![
            MonitorInfo { index: 0, name: "a".into(), x: 0, y: 0, width: 1920, height: 1080, scale: 1.5, primary: true, current: true },
            MonitorInfo { index: 1, name: "b".into(), x: 1920, y: 0, width: 2560, height: 1440, scale: 1.0, primary: false, current: false },
        ];
        assert!(on_some_monitor(&ms, 100, 100));
        assert!(on_some_monitor(&ms, 3000, 100));
        assert!(!on_some_monitor(&ms, 5000, 100));
        assert!(!on_some_monitor(&ms, -10, 100));
        assert_eq!(corner(&ms[0]), (60, 60));
    }
}
