//! DesktopGadget の本体（Rust側）
//!
//! - sysmon:   CPU・GPU・RAM・温度・ネットワークの取得（PDH、DXGI、D3DKMT、IP Helper）
//! - sysstate: 電源・ゲーム中・ロックの検知（更新間隔の切り替えに使う）
//! - weather:  天気・警報・雨雲レーダーの取得（許可リストとキャッシュつき）
//! - resident: 常駐化（デスクトップに付ける、位置、設定ファイル、自動起動、トレイ）
//! - lib.rs:   画面（WebView）から呼ぶコマンドと、擬似ガラス用の壁紙の読み込み

mod sysmon;
mod sysstate;
mod resident;
mod weather;

use base64::Engine as _;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, WebviewWindow};
/// 窓の大きさを、カードの大きさに合わせる（画面側から、論理ピクセルで指定する）
#[tauri::command]
fn fit_window(window: WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------
// 擬似ガラス用：壁紙とモニターの情報
// ---------------------------------------------------------------

/// モニターと窓の位置（すべて物理ピクセル）
#[derive(Serialize)]
struct Geometry {
    mon_x: i32,
    mon_y: i32,
    mon_w: u32,
    mon_h: u32,
    scale: f64,
    win_x: i32,
    win_y: i32,
}

fn geometry_of(window: &WebviewWindow) -> Result<Geometry, String> {
    let mon = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("モニターの情報を取得できませんでした")?;
    let pos = window.inner_position().map_err(|e| e.to_string())?;
    Ok(Geometry {
        mon_x: mon.position().x,
        mon_y: mon.position().y,
        mon_w: mon.size().width,
        mon_h: mon.size().height,
        scale: mon.scale_factor(),
        win_x: pos.x,
        win_y: pos.y,
    })
}

#[tauri::command]
fn get_window_geometry(window: WebviewWindow) -> Result<Geometry, String> {
    geometry_of(&window)
}

/// 壁紙の画像（data URL）と、Windowsの「ページ表示」の設定
#[derive(Serialize)]
struct Wallpaper {
    image: String,
    path: String,
    /// 10=ページ全体に合わせて拡大（塗りつぶし）, 6=調整, 2=拡大して表示, 0=中央, 22=スパン
    style: u32,
    tile: bool,
    /// 背景色（RGB）。「調整」「中央」のときの余白の色
    bg: [u8; 3],
}

#[cfg(windows)]
fn read_wallpaper_settings() -> Result<(String, u32, bool, [u8; 3]), String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let desk = hkcu
        .open_subkey("Control Panel\\Desktop")
        .map_err(|e| e.to_string())?;
    let path: String = desk.get_value("WallPaper").map_err(|e| e.to_string())?;
    let style: u32 = desk
        .get_value::<String, _>("WallpaperStyle")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(10);
    let tile: bool = desk
        .get_value::<String, _>("TileWallpaper")
        .ok()
        .map(|s| s.trim() == "1")
        .unwrap_or(false);
    let bg: [u8; 3] = hkcu
        .open_subkey("Control Panel\\Colors")
        .ok()
        .and_then(|k| k.get_value::<String, _>("Background").ok())
        .map(|s| {
            let v: Vec<u8> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() == 3 {
                [v[0], v[1], v[2]]
            } else {
                [0, 0, 0]
            }
        })
        .unwrap_or([0, 0, 0]);
    Ok((path, style, tile, bg))
}

#[cfg(not(windows))]
fn read_wallpaper_settings() -> Result<(String, u32, bool, [u8; 3]), String> {
    Err("Windows専用の機能です".into())
}

fn sniff_mime(b: &[u8]) -> &'static str {
    if b.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if b.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if b.starts_with(b"BM") {
        "image/bmp"
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

#[tauri::command]
fn get_wallpaper() -> Result<Wallpaper, String> {
    let (path, style, tile, bg) = read_wallpaper_settings()?;
    if path.trim().is_empty() {
        return Err("壁紙が設定されていません（単色の背景です）".into());
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("壁紙を読めませんでした（{}）: {}", path, e))?;
    let mime = sniff_mime(&bytes);
    let image = format!(
        "data:{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    Ok(Wallpaper {
        image,
        path,
        style,
        tile,
        bg,
    })
}

// ---------------------------------------------------------------
// フェーズ3：使用量（§5）と動作状態（§7）
// ---------------------------------------------------------------

/// 使用量の取得は、前回からの差で計算するため、1つの取得器を使い回す
struct UsageState(Mutex<Option<sysmon::SysMon>>);

fn with_mon<T>(state: &UsageState, f: impl FnOnce(&mut sysmon::SysMon) -> T) -> Result<T, String> {
    let mut g = state.0.lock().map_err(|_| "使用量の取得器が壊れました".to_string())?;
    let mon = g.get_or_insert_with(sysmon::SysMon::new);
    Ok(f(mon))
}

/// CPU・GPU・RAMの使用量。画面側が更新間隔（1〜5秒、バッテリー時は倍）で呼ぶ
#[tauri::command]
async fn get_usage(state: tauri::State<'_, UsageState>) -> Result<sysmon::Usage, String> {
    with_mon(&state, |m| m.sample())
}

/// GPUの一覧を取り直す（設定の「表示するGPU」用）
#[tauri::command]
async fn list_gpus(state: tauri::State<'_, UsageState>) -> Result<Vec<sysmon::GpuUsage>, String> {
    with_mon(&state, |m| {
        m.refresh_adapters();
        m.sample().gpus
    })
}

/// 電源・ゲーム中・ロックの状態。game_exes は「ゲーム扱いにするアプリ」の実行ファイル名
#[tauri::command]
async fn get_system_state(game_exes: Vec<String>) -> Result<sysstate::SystemState, String> {
    Ok(sysstate::read(&game_exes))
}

// ---------------------------------------------------------------
// フェーズ4：天気・警報・雨雲レーダー（§4）。通信は weather.rs にまとめ、許可リストの取得先だけに接続する
// ---------------------------------------------------------------

type NetState<'a> = tauri::State<'a, weather::Net>;

#[tauri::command]
async fn wx_geocode(net: NetState<'_>, query: String) -> Result<Vec<weather::Place>, String> {
    weather::geocode(&net, &query).await
}

#[tauri::command]
async fn wx_current(net: NetState<'_>, lat: f64, lon: f64) -> Result<weather::Weather, String> {
    weather::weather(&net, lat, lon).await
}

/// 地点から、警報の地域（市区町村）を推定する
#[tauri::command]
async fn wx_find_area(net: NetState<'_>, lat: f64, lon: f64) -> Result<weather::AreaInfo, String> {
    weather::find_area(&net, lat, lon).await
}

/// 保存してある市区町村コードの名前と、選び直し用の一覧
#[tauri::command]
async fn wx_area_info(net: NetState<'_>, code: String) -> Result<weather::AreaInfo, String> {
    weather::area_info(&net, &code).await
}

#[tauri::command]
async fn wx_warnings(net: NetState<'_>, code: String) -> Result<weather::Warnings, String> {
    weather::warnings(&net, &code).await
}

#[tauri::command]
async fn wx_radar_times(net: NetState<'_>) -> Result<weather::RadarTimes, String> {
    weather::radar_times(&net).await
}

/// 雨雲のタイル（PNG）。画面側には ArrayBuffer で渡す
#[tauri::command]
async fn wx_radar_tile(net: NetState<'_>, basetime: String, validtime: String, z: u8, x: u32, y: u32) -> Result<tauri::ipc::Response, String> {
    let b = weather::radar_tile(&net, &basetime, &validtime, z, x, y).await?;
    Ok(tauri::ipc::Response::new(b.as_ref().clone()))
}

/// 背景の地図タイル（地理院タイル）
#[tauri::command]
async fn wx_base_tile(net: NetState<'_>, z: u8, x: u32, y: u32) -> Result<tauri::ipc::Response, String> {
    let b = weather::base_tile(&net, z, x, y).await?;
    Ok(tauri::ipc::Response::new(b.as_ref().clone()))
}

#[derive(Serialize)]
struct SourcesInfo {
    path: String,
    note: Option<String>,
}

/// 取得先の設定ファイルの場所（設定画面に出す）
#[tauri::command]
fn wx_sources(net: NetState<'_>) -> SourcesInfo {
    SourcesInfo { path: net.src_path.clone(), note: net.src_note.clone() }
}

// ---------------------------------------------------------------
// フェーズ5：常駐化（デスクトップに付ける、位置、設定ファイル、自動起動、設定画面）
// ---------------------------------------------------------------

/// "attach"（デスクトップに付ける。Win+Dでも残る）/ "normal"（通常の最背面の窓）
#[tauri::command]
fn set_desktop_mode(app: tauri::AppHandle, mode: String) -> Result<(), String> {
    resident::apply_desktop_mode(&app, mode != "normal")
}

#[tauri::command]
async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    resident::open_settings(&app)
}

#[tauri::command]
fn settings_read(app: tauri::AppHandle) -> Option<String> {
    resident::settings_read(&app)
}

#[tauri::command]
fn settings_write(app: tauri::AppHandle, json: String) -> Result<(), String> {
    resident::settings_write(&app, &json)
}

/// 設定の書き出し。保存先をダイアログで選ぶ。戻り値は保存したパス（取り消しは None）
#[tauri::command]
async fn settings_export(app: tauri::AppHandle, json: String) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    resident::valid_settings(&json)?;
    let Some(fp) = app
        .dialog()
        .file()
        .add_filter("設定ファイル（JSON）", &["json"])
        .set_file_name("desktop-gadget-settings.json")
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let pretty = serde_json::from_str::<serde_json::Value>(&json)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or(json);
    std::fs::write(&path, pretty).map_err(|e| format!("書き出せませんでした: {}", e))?;
    Ok(Some(path.display().to_string()))
}

/// 設定の読み込み。ファイルをダイアログで選び、内容を確かめて保存する。戻り値は設定のJSON（取り消しは None）
#[tauri::command]
async fn settings_import(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(fp) = app.dialog().file().add_filter("設定ファイル（JSON）", &["json"]).blocking_pick_file() else {
        return Ok(None);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("読めませんでした: {}", e))?;
    resident::valid_settings(&text)?;
    // 1行のJSONにそろえて保存する
    let compact = serde_json::from_str::<serde_json::Value>(&text).map(|v| v.to_string()).map_err(|e| e.to_string())?;
    resident::settings_write(&app, &compact)?;
    Ok(Some(compact))
}

#[tauri::command]
fn settings_dir(app: tauri::AppHandle) -> String {
    resident::settings_dir(&app)
}

#[tauri::command]
fn autostart_get() -> Result<resident::AutoStart, String> {
    resident::autostart_get()
}

#[tauri::command]
fn autostart_set(on: bool) -> Result<resident::AutoStart, String> {
    resident::autostart_set(on)
}

#[tauri::command]
fn list_monitors(app: tauri::AppHandle) -> Vec<resident::MonitorInfo> {
    resident::monitors(&app)
}

#[tauri::command]
fn move_to_monitor(app: tauri::AppHandle, index: usize) -> Result<(), String> {
    resident::move_to_monitor(&app, index)
}

/// 座標を指定して動かす（物理ピクセル）
#[tauri::command]
fn move_window(app: tauri::AppHandle, x: i32, y: i32) -> Result<(), String> {
    resident::move_main(&app, x, y)
}

#[tauri::command]
fn reset_position(app: tauri::AppHandle) -> Result<(), String> {
    resident::reset_position(&app)
}

/// 壁紙が変わったかを見るための目印（パスと更新時刻）。画面側が30秒ごとに比べる
#[tauri::command]
fn wallpaper_stamp() -> Result<String, String> {
    let (path, style, tile, _) = read_wallpaper_settings()?;
    let t = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(format!("{}|{}|{}|{}", path, style, tile, t))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 二重起動の防止は、ほかのプラグインより先に登録する。2回目に起動したら、設定画面を開く
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            resident::open_settings_later(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(UsageState(Mutex::new(None)))
        .on_window_event(|window, event| {
            // ガジェットを動かしたら、位置を覚える（保存は見張りのスレッドが、止まってから行う）
            if let tauri::WindowEvent::Moved(p) = event {
                if window.label() == "main" {
                    if let Some(st) = window.app_handle().try_state::<resident::Resident>() {
                        st.note_move(p.x, p.y);
                    }
                }
            }
        })
        .setup(|app| {
            // 常駐化: 位置の復元、デスクトップに付ける、トレイ、見張り
            let dir = app.path().app_config_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            app.manage(resident::Resident::new(dir));
            resident::restore_position(app.handle());
            if let Err(e) = resident::apply_desktop_mode(app.handle(), true) {
                eprintln!("デスクトップに付けられませんでした: {}", e);
            }
            resident::build_tray(app)?;
            resident::spawn_watcher(app.handle().clone());

            // 取得先の設定ファイル（%APPDATA%\local.desktopgadget\sources.json）
            let path = app
                .path()
                .app_config_dir()
                .map(|d| d.join("sources.json"))
                .unwrap_or_else(|_| std::path::PathBuf::from("sources.json"));
            let (src, note) = weather::load_sources(&path);
            app.manage(weather::Net::new(src, path.display().to_string(), note));

            // 使用率は「前回からの差」なので、起動時に1回目を取っておく（最初の表示までの待ちを短くする）
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let st = handle.state::<UsageState>();
                let _ = with_mon(&st, |_| ());
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            fit_window,
            get_window_geometry,
            get_wallpaper,
            get_usage,
            list_gpus,
            get_system_state,
            wx_geocode,
            wx_current,
            wx_find_area,
            wx_area_info,
            wx_warnings,
            wx_radar_times,
            wx_radar_tile,
            wx_base_tile,
            wx_sources,
            set_desktop_mode,
            open_settings,
            settings_read,
            settings_write,
            settings_export,
            settings_import,
            settings_dir,
            autostart_get,
            autostart_set,
            list_monitors,
            move_to_monitor,
            move_window,
            reset_position,
            wallpaper_stamp
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
