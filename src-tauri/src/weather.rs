//! フェーズ4：天気（§4）
//!
//! - 気温・湿度・天気: Open-Meteo（非商用の無料枠。CC BY 4.0）
//! - 地名検索: Open-Meteo Geocoding
//! - 警報・注意報: 気象庁ホームページのJSON（令和8年体系 r8）
//! - 雨雲レーダー: 気象庁 高解像度降水ナウキャストのタイル、背景は地理院タイル
//! - 地点→市区町村: 地理院の逆ジオコーダー（警報の地域の推定にだけ使う）
//!
//! 通信はすべてここで行い、画面には結果だけを渡す。取得先は設定ファイル sources.json に切り出し、
//! そこに書かれたホストだけに接続する（許可リスト）。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------
// 取得先（sources.json）
// ---------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Sources {
    pub open_meteo_forecast: String,
    pub open_meteo_geocoding: String,
    pub jma_area: String,
    pub jma_warning: String,
    pub jma_times_past: String,
    pub jma_times_forecast: String,
    pub jma_radar_tile: String,
    pub radar_zooms: Vec<u8>,
    pub base_tile: String,
    pub base_zoom_min: u8,
    pub base_zoom_max: u8,
    pub reverse_geocoder: String,
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            open_meteo_forecast: "https://api.open-meteo.com/v1/forecast".into(),
            open_meteo_geocoding: "https://geocoding-api.open-meteo.com/v1/search".into(),
            jma_area: "https://www.jma.go.jp/bosai/common/const/area.json".into(),
            jma_warning: "https://www.jma.go.jp/bosai/warning/data/r8/{office}.json".into(),
            jma_times_past: "https://www.jma.go.jp/bosai/jmatile/data/nowc/targetTimes_N1.json".into(),
            jma_times_forecast: "https://www.jma.go.jp/bosai/jmatile/data/nowc/targetTimes_N2.json".into(),
            jma_radar_tile: "https://www.jma.go.jp/bosai/jmatile/data/nowc/{basetime}/none/{validtime}/surf/hrpns/{z}/{x}/{y}.png"
                .into(),
            radar_zooms: vec![4, 6, 8, 10],
            base_tile: "https://cyberjapandata.gsi.go.jp/xyz/pale/{z}/{x}/{y}.png".into(),
            base_zoom_min: 5,
            base_zoom_max: 18,
            reverse_geocoder: "https://mreversegeocoder.gsi.go.jp/reverse-geocoder/LonLatToAddress?lat={lat}&lon={lon}".into(),
        }
    }
}

impl Sources {
    fn all_urls(&self) -> Vec<&str> {
        vec![
            &self.open_meteo_forecast,
            &self.open_meteo_geocoding,
            &self.jma_area,
            &self.jma_warning,
            &self.jma_times_past,
            &self.jma_times_forecast,
            &self.jma_radar_tile,
            &self.base_tile,
            &self.reverse_geocoder,
        ]
    }
}

/// URLのホスト名を取り出す（"https://host/..." → "host"）
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// sources.json を読む。なければ既定値で作る。読めないときは既定値を使い、理由を返す
pub fn load_sources(path: &std::path::Path) -> (Sources, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<Sources>(&s) {
            Ok(src) => (src, None),
            Err(e) => (Sources::default(), Some(format!("取得先の設定（{}）を読めないため、既定値を使います: {}", path.display(), e))),
        },
        Err(_) => {
            let src = Sources::default();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, serde_json::to_string_pretty(&src).unwrap_or_default());
            (src, None)
        }
    }
}

// ---------------------------------------------------------------
// 通信（許可リストとキャッシュ）
// ---------------------------------------------------------------

/// タイルのキャッシュ（URL → PNG）。古いものから捨てる
struct TileCache {
    map: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
    cap: usize,
}

impl TileCache {
    fn get(&self, k: &str) -> Option<Arc<Vec<u8>>> {
        self.map.get(k).cloned()
    }
    fn put(&mut self, k: String, v: Arc<Vec<u8>>) {
        if self.map.insert(k.clone(), v).is_none() {
            self.order.push_back(k);
        }
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

pub struct Net {
    client: reqwest::Client,
    pub src: Sources,
    pub src_path: String,
    pub src_note: Option<String>,
    allowed: Vec<String>,
    tiles: Mutex<TileCache>,
    area: Mutex<Option<(Instant, Arc<Value>)>>,
}

impl Net {
    pub fn new(src: Sources, src_path: String, src_note: Option<String>) -> Self {
        let mut allowed: Vec<String> = src.all_urls().into_iter().filter_map(host_of).collect();
        allowed.sort();
        allowed.dedup();
        let client = reqwest::Client::builder()
            .user_agent(concat!("DesktopGadget/", env!("CARGO_PKG_VERSION"), " (personal desktop widget)"))
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Net {
            client,
            src,
            src_path,
            src_note,
            allowed,
            tiles: Mutex::new(TileCache { map: HashMap::new(), order: VecDeque::new(), cap: 1500 }),
            area: Mutex::new(None),
        }
    }

    fn check(&self, url: &str) -> Result<(), String> {
        if !url.starts_with("https://") {
            return Err(format!("https以外には接続しません: {}", url));
        }
        match host_of(url) {
            Some(h) if self.allowed.contains(&h) => Ok(()),
            _ => Err(format!("許可リストにない取得先です: {}", url)),
        }
    }

    async fn get_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        self.check(url)?;
        let r = self.client.get(url).send().await.map_err(|e| net_err(url, &e))?;
        let st = r.status();
        if !st.is_success() {
            return Err(format!("{}（HTTP {}）", short_url(url), st.as_u16()));
        }
        r.bytes().await.map(|b| b.to_vec()).map_err(|e| net_err(url, &e))
    }

    async fn get_json(&self, url: &str) -> Result<Value, String> {
        let b = self.get_bytes(url).await?;
        serde_json::from_slice(&b).map_err(|e| format!("{} の形式が想定と違います: {}", short_url(url), e))
    }

    /// タイル（キャッシュつき）。cache=false のときも結果はキャッシュに入れる
    async fn tile(&self, url: String) -> Result<Arc<Vec<u8>>, String> {
        if let Some(v) = self.tiles.lock().ok().and_then(|c| c.get(&url)) {
            return Ok(v);
        }
        let b = Arc::new(self.get_bytes(&url).await?);
        if let Ok(mut c) = self.tiles.lock() {
            c.put(url, b.clone());
        }
        Ok(b)
    }

    /// 気象庁の地域コード表（1日キャッシュ）
    async fn area_json(&self) -> Result<Arc<Value>, String> {
        if let Some((t, v)) = self.area.lock().ok().and_then(|g| g.clone()) {
            if t.elapsed() < Duration::from_secs(24 * 3600) {
                return Ok(v);
            }
        }
        let v = Arc::new(self.get_json(&self.src.jma_area).await?);
        if v.get("class20s").and_then(|x| x.as_object()).is_none() {
            return Err("気象庁の地域コード表の形式が想定と違います（class20s がありません）".into());
        }
        if let Ok(mut g) = self.area.lock() {
            *g = Some((Instant::now(), v.clone()));
        }
        Ok(v)
    }
}

fn short_url(url: &str) -> String {
    let u = url.split('?').next().unwrap_or(url);
    if u.len() > 90 {
        format!("{}…", &u[..90])
    } else {
        u.to_string()
    }
}

fn net_err(url: &str, e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "時間切れ"
    } else if e.is_connect() {
        "接続できません（オフラインかもしれません）"
    } else {
        "通信エラー"
    };
    format!("{}: {}（{}）", kind, short_url(url), e)
}

/// URLのパラメーターに入れる文字列を、最小限エスケープする
fn qenc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{:02X}", b)),
        }
    }
    o
}

fn fill(t: &str, kv: &[(&str, String)]) -> String {
    let mut s = t.to_string();
    for (k, v) in kv {
        s = s.replace(&format!("{{{}}}", k), v);
    }
    s
}

// ---------------------------------------------------------------
// 気温・湿度（Open-Meteo）
// ---------------------------------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Weather {
    pub temp_c: Option<f64>,
    pub humidity: Option<f64>,
    /// WMOの天気コード
    pub code: Option<i64>,
    pub is_day: bool,
    /// 観測（モデル）時刻。現地時刻 "2026-11-04T01:00"
    pub time: Option<String>,
}

pub fn parse_weather(v: &Value) -> Result<Weather, String> {
    let c = v.get("current").ok_or("天気の応答に current がありません")?;
    Ok(Weather {
        temp_c: c.get("temperature_2m").and_then(|x| x.as_f64()),
        humidity: c.get("relative_humidity_2m").and_then(|x| x.as_f64()),
        code: c.get("weather_code").and_then(|x| x.as_i64()),
        is_day: c.get("is_day").and_then(|x| x.as_i64()).map(|d| d == 1).unwrap_or(true),
        time: c.get("time").and_then(|x| x.as_str()).map(|s| s.to_string()),
    })
}

pub async fn weather(net: &Net, lat: f64, lon: f64) -> Result<Weather, String> {
    check_latlon(lat, lon)?;
    let url = format!(
        "{}?latitude={:.4}&longitude={:.4}&current=temperature_2m,relative_humidity_2m,weather_code,is_day&timezone=auto",
        net.src.open_meteo_forecast, lat, lon
    );
    parse_weather(&net.get_json(&url).await?)
}

fn check_latlon(lat: f64, lon: f64) -> Result<(), String> {
    if lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
        Ok(())
    } else {
        Err("緯度・経度の値が正しくありません".into())
    }
}

// ---------------------------------------------------------------
// 地名検索（Open-Meteo Geocoding）
// ---------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct Place {
    pub name: String,
    /// 「東京都 千代田区」など、候補を見分けるための説明
    pub detail: String,
    pub lat: f64,
    pub lon: f64,
    pub country_code: String,
}

pub fn parse_places(v: &Value) -> Vec<Place> {
    let Some(arr) = v.get("results").and_then(|x| x.as_array()) else { return Vec::new() };
    arr.iter()
        .filter_map(|r| {
            let s = |k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let lat = r.get("latitude")?.as_f64()?;
            let lon = r.get("longitude")?.as_f64()?;
            let name = s("name");
            let mut parts: Vec<String> = Vec::new();
            for k in ["admin1", "admin2", "admin3"] {
                let p = s(k);
                if !p.is_empty() && p != name && !parts.contains(&p) {
                    parts.push(p);
                }
            }
            let country = s("country");
            if s("country_code") != "JP" && !country.is_empty() {
                parts.push(country);
            }
            Some(Place { name, detail: parts.join(" "), lat, lon, country_code: s("country_code") })
        })
        .collect()
}

pub async fn geocode(net: &Net, query: &str) -> Result<Vec<Place>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    if q.chars().count() > 60 {
        return Err("地名が長すぎます".into());
    }
    let url = format!("{}?name={}&count=10&language=ja&format=json", net.src.open_meteo_geocoding, qenc(q));
    Ok(parse_places(&net.get_json(&url).await?))
}

// ---------------------------------------------------------------
// 警報の地域（市区町村）
// ---------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct AreaOption {
    pub code: String,
    pub name: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct AreaInfo {
    /// 市区町村（気象庁の二次細分区域）コード 7桁
    pub code: String,
    pub name: String,
    /// 都道府県（府県予報区）のコードと名前
    pub office: String,
    pub office_name: String,
    /// 同じ府県予報区の市区町村（設定で選び直すため）
    pub options: Vec<AreaOption>,
    pub note: Option<String>,
}

/// class20 → class15 → class10 → offices と親をたどって、府県予報区のコードを出す
pub fn office_of(area: &Value, class20: &str) -> Option<String> {
    let p15 = area.get("class20s")?.get(class20)?.get("parent")?.as_str()?;
    let p10 = area.get("class15s")?.get(p15)?.get("parent")?.as_str()?;
    let off = area.get("class10s")?.get(p10)?.get("parent")?.as_str()?;
    Some(off.to_string())
}

/// 市区町村コード（5桁）から、気象庁の二次細分区域コードを選ぶ。
/// 1) 先頭5桁が同じもの。2) 政令市の区などで無いときは、同じ都道府県の中で「それ以下で最大」のもの（区 → 市）
pub fn match_class20(area: &Value, muni5: &str) -> Vec<String> {
    let Some(c20) = area.get("class20s").and_then(|x| x.as_object()) else { return Vec::new() };
    let mut keys: Vec<&String> = c20.keys().collect();
    keys.sort();
    let exact: Vec<String> = keys.iter().filter(|k| k.starts_with(muni5)).map(|k| k.to_string()).collect();
    if !exact.is_empty() {
        return exact;
    }
    let Ok(m) = muni5.parse::<u32>() else { return Vec::new() };
    let pref = &muni5[..2];
    let mut best: Option<(u32, &String)> = None;
    for k in keys.iter().filter(|k| k.len() >= 5 && k.starts_with(pref)) {
        if let Ok(v) = k[..5].parse::<u32>() {
            if v <= m && best.map(|(b, _)| v > b).unwrap_or(true) {
                best = Some((v, k));
            }
        }
    }
    match best {
        Some((v, _)) => keys
            .iter()
            .filter(|k| k.len() >= 5 && k.starts_with(pref) && k[..5].parse::<u32>().ok() == Some(v))
            .map(|k| k.to_string())
            .collect(),
        None => Vec::new(),
    }
}

pub fn area_info_from(area: &Value, code: &str, note: Option<String>) -> Result<AreaInfo, String> {
    let c20 = area.get("class20s").and_then(|x| x.as_object()).ok_or("地域コード表が読めません")?;
    let name = c20
        .get(code)
        .and_then(|x| x.get("name"))
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("市区町村コード {} が見つかりません", code))?
        .to_string();
    let office = office_of(area, code).ok_or("府県予報区をたどれませんでした")?;
    let office_name = area
        .get("offices")
        .and_then(|o| o.get(&office))
        .and_then(|x| x.get("name"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let mut options: Vec<AreaOption> = c20
        .iter()
        .filter(|(k, _)| office_of(area, k).as_deref() == Some(office.as_str()))
        .map(|(k, v)| AreaOption { code: k.clone(), name: v.get("name").and_then(|x| x.as_str()).unwrap_or(k).to_string() })
        .collect();
    options.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(AreaInfo { code: code.to_string(), name, office, office_name, options, note })
}

pub async fn find_area(net: &Net, lat: f64, lon: f64) -> Result<AreaInfo, String> {
    check_latlon(lat, lon)?;
    let url = fill(&net.src.reverse_geocoder, &[("lat", format!("{:.6}", lat)), ("lon", format!("{:.6}", lon))]);
    let rg = net.get_json(&url).await?;
    let muni = rg
        .get("results")
        .and_then(|r| r.get("muniCd"))
        .and_then(|m| m.as_str().map(|s| s.to_string()).or_else(|| m.as_i64().map(|n| n.to_string())))
        .ok_or("この地点の市区町村がわかりません（海の上や日本の外かもしれません）")?;
    let muni5 = format!("{:0>5}", muni.trim());
    let area = net.area_json().await?;
    let cands = match_class20(&area, &muni5);
    let first = cands.first().ok_or_else(|| format!("市区町村コード {} に対応する気象庁の地域が見つかりません", muni5))?;
    let note = if cands.len() > 1 { Some("この市区町村は、気象庁の区域が分かれています。設定で選び直せます".to_string()) } else { None };
    area_info_from(&area, first, note)
}

pub async fn area_info(net: &Net, code: &str) -> Result<AreaInfo, String> {
    if code.len() != 7 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err("市区町村コードは7桁の数字です".into());
    }
    let area = net.area_json().await?;
    area_info_from(&area, code, None)
}

// ---------------------------------------------------------------
// 警報・注意報（令和8年体系）
// ---------------------------------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct WarnItem {
    pub code: String,
    pub name: String,
    /// "special"（特別警報）/ "danger"（危険警報）/ "warning"（警報）/ "advisory"（注意報）
    pub kind: String,
    /// 並べ順（小さいほど重い）
    pub rank: u8,
}

#[derive(Serialize, Clone, Debug)]
pub struct Warnings {
    pub items: Vec<WarnItem>,
    /// 最新の発表時刻（"2026-06-06T13:30:00+09:00"）
    pub report_time: Option<String>,
    pub office_name: String,
    pub area_name: String,
}

/// 令和8年体系の警報・注意報コード（2026年5月29日以降）
pub fn warn_name(code: &str) -> Option<(&'static str, &'static str)> {
    Some(match code {
        "32" => ("暴風雪特別警報", "special"),
        "33" => ("レベル5大雨特別警報", "special"),
        "34" => ("レベル5氾濫特別警報", "special"),
        "35" => ("暴風特別警報", "special"),
        "36" => ("大雪特別警報", "special"),
        "37" => ("波浪特別警報", "special"),
        "38" => ("レベル5高潮特別警報", "special"),
        "39" => ("レベル5土砂災害特別警報", "special"),
        "43" => ("レベル4大雨危険警報", "danger"),
        "44" => ("レベル4氾濫危険警報", "danger"),
        "48" => ("レベル4高潮危険警報", "danger"),
        "49" => ("レベル4土砂災害危険警報", "danger"),
        "02" => ("暴風雪警報", "warning"),
        "03" => ("レベル3大雨警報", "warning"),
        "04" => ("レベル3氾濫警報", "warning"),
        "05" => ("暴風警報", "warning"),
        "06" => ("大雪警報", "warning"),
        "07" => ("波浪警報", "warning"),
        "08" => ("レベル3高潮警報", "warning"),
        "09" => ("レベル3土砂災害警報", "warning"),
        "10" => ("レベル2大雨注意報", "advisory"),
        "12" => ("大雪注意報", "advisory"),
        "13" => ("風雪注意報", "advisory"),
        "14" => ("雷注意報", "advisory"),
        "15" => ("強風注意報", "advisory"),
        "16" => ("波浪注意報", "advisory"),
        "17" => ("融雪注意報", "advisory"),
        "18" => ("レベル2氾濫注意報", "advisory"),
        "19" => ("レベル2高潮注意報", "advisory"),
        "20" => ("濃霧注意報", "advisory"),
        "21" => ("乾燥注意報", "advisory"),
        "22" => ("なだれ注意報", "advisory"),
        "23" => ("低温注意報", "advisory"),
        "24" => ("霜注意報", "advisory"),
        "25" => ("着氷注意報", "advisory"),
        "26" => ("着雪注意報", "advisory"),
        "27" => ("その他の注意報", "advisory"),
        _ => return None,
    })
}

fn rank_of(kind: &str) -> u8 {
    match kind {
        "special" => 0,
        "danger" => 1,
        "warning" => 2,
        _ => 3,
    }
}

/// 府県のJSON（電文ごとの配列）から、指定の市区町村で発表中のものを取り出す
pub fn parse_warnings(v: &Value, class20: &str) -> (Vec<WarnItem>, Option<String>) {
    let mut out: Vec<WarnItem> = Vec::new();
    let mut latest: Option<String> = None;
    let reports: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        Value::Object(_) => vec![v],
        _ => Vec::new(),
    };
    for rep in reports {
        if let Some(t) = rep.get("reportDatetime").and_then(|x| x.as_str()) {
            if latest.as_deref().map(|l| t > l).unwrap_or(true) {
                latest = Some(t.to_string());
            }
        }
        let Some(items) = rep.get("warning").and_then(|w| w.get("class20Items")).and_then(|x| x.as_array()) else { continue };
        for it in items.iter().filter(|it| it.get("areaCode").and_then(|x| x.as_str()) == Some(class20)) {
            let Some(kinds) = it.get("kinds").and_then(|x| x.as_array()) else { continue };
            for k in kinds {
                let status = k.get("status").and_then(|x| x.as_str()).unwrap_or("");
                if status == "解除" || status.contains("発表警報・注意報はなし") {
                    continue;
                }
                let Some(code) = k.get("code").and_then(|x| x.as_str()) else { continue };
                if code == "00" || out.iter().any(|o| o.code == code) {
                    continue;
                }
                let (name, kind) = warn_name(code).unwrap_or(("その他の警報・注意報", "advisory"));
                out.push(WarnItem { code: code.to_string(), name: name.to_string(), kind: kind.to_string(), rank: rank_of(kind) });
            }
        }
    }
    out.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.code.cmp(&b.code)));
    (out, latest)
}

pub async fn warnings(net: &Net, class20: &str) -> Result<Warnings, String> {
    let info = area_info(net, class20).await?;
    let url = fill(&net.src.jma_warning, &[("office", info.office.clone())]);
    let v = net.get_json(&url).await?;
    let (items, report_time) = parse_warnings(&v, class20);
    Ok(Warnings { items, report_time, office_name: info.office_name, area_name: info.name })
}

// ---------------------------------------------------------------
// 雨雲レーダー
// ---------------------------------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Frame {
    pub basetime: String,
    pub validtime: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct RadarTimes {
    /// 実況（古い順）
    pub past: Vec<Frame>,
    /// 予測（早い順）
    pub forecast: Vec<Frame>,
    pub zooms: Vec<u8>,
    /// 予測が取れなかったときの理由
    pub note: Option<String>,
}

fn is_stamp(s: &str) -> bool {
    s.len() == 14 && s.bytes().all(|b| b.is_ascii_digit())
}

pub fn parse_times(v: &Value) -> Vec<Frame> {
    let mut out: Vec<Frame> = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let b = e.get("basetime")?.as_str()?;
                    let t = e.get("validtime")?.as_str()?;
                    let els = e.get("elements").and_then(|x| x.as_array());
                    let has = els.map(|l| l.iter().any(|x| x.as_str() == Some("hrpns"))).unwrap_or(true);
                    if is_stamp(b) && is_stamp(t) && has {
                        Some(Frame { basetime: b.into(), validtime: t.into() })
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.validtime.cmp(&b.validtime));
    out.dedup_by(|a, b| a.validtime == b.validtime);
    out
}

pub async fn radar_times(net: &Net) -> Result<RadarTimes, String> {
    let past = parse_times(&net.get_json(&net.src.jma_times_past).await?);
    if past.is_empty() {
        return Err("雨雲レーダーの時刻一覧が空です（形式が変わったかもしれません）".into());
    }
    let (forecast, note) = match net.get_json(&net.src.jma_times_forecast).await {
        Ok(v) => {
            let last = past.last().map(|f| f.validtime.clone()).unwrap_or_default();
            (parse_times(&v).into_iter().filter(|f| f.validtime > last).collect(), None)
        }
        Err(e) => (Vec::new(), Some(format!("予測を取得できませんでした: {}", e))),
    };
    Ok(RadarTimes { past, forecast, zooms: net.src.radar_zooms.clone(), note })
}

fn check_tile(z: u8, x: u32, y: u32) -> Result<(), String> {
    if z > 18 {
        return Err("ズームが大きすぎます".into());
    }
    let n = 1u32 << z;
    if x >= n || y >= n {
        return Err("タイルの番号が範囲外です".into());
    }
    Ok(())
}

pub async fn radar_tile(net: &Net, basetime: &str, validtime: &str, z: u8, x: u32, y: u32) -> Result<Arc<Vec<u8>>, String> {
    if !is_stamp(basetime) || !is_stamp(validtime) {
        return Err("時刻の形式が正しくありません".into());
    }
    if !net.src.radar_zooms.contains(&z) {
        return Err(format!("このズーム（{}）の雨雲タイルはありません", z));
    }
    check_tile(z, x, y)?;
    let url = fill(
        &net.src.jma_radar_tile,
        &[
            ("basetime", basetime.into()),
            ("validtime", validtime.into()),
            ("z", z.to_string()),
            ("x", x.to_string()),
            ("y", y.to_string()),
        ],
    );
    net.tile(url).await
}

pub async fn base_tile(net: &Net, z: u8, x: u32, y: u32) -> Result<Arc<Vec<u8>>, String> {
    if z < net.src.base_zoom_min || z > net.src.base_zoom_max {
        return Err(format!("このズーム（{}）の地図タイルはありません", z));
    }
    check_tile(z, x, y)?;
    let url = fill(&net.src.base_tile, &[("z", z.to_string()), ("x", x.to_string()), ("y", y.to_string())]);
    net.tile(url).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hosts() {
        assert_eq!(host_of("https://www.jma.go.jp/bosai/x.json").as_deref(), Some("www.jma.go.jp"));
        assert_eq!(host_of("https://api.open-meteo.com/v1/forecast?x=1").as_deref(), Some("api.open-meteo.com"));
        assert_eq!(host_of("ftp://x"), None);
        let n = Net::new(Sources::default(), String::new(), None);
        assert!(n.check("https://cyberjapandata.gsi.go.jp/xyz/pale/8/1/1.png").is_ok());
        assert!(n.check("https://evil.example.com/").is_err());
        assert!(n.check("http://www.jma.go.jp/").is_err());
    }

    #[test]
    fn weather_parse() {
        let v = json!({"current":{"time":"2026-11-04T01:00","temperature_2m":12.3,"relative_humidity_2m":71,"weather_code":3,"is_day":0}});
        let w = parse_weather(&v).unwrap();
        assert_eq!(w.temp_c, Some(12.3));
        assert_eq!(w.humidity, Some(71.0));
        assert_eq!(w.code, Some(3));
        assert!(!w.is_day);
        assert!(parse_weather(&json!({})).is_err());
    }

    #[test]
    fn places() {
        let v = json!({"results":[
            {"name":"千代田区","latitude":35.69,"longitude":139.75,"country":"日本","country_code":"JP","admin1":"東京都"},
            {"name":"Paris","latitude":48.85,"longitude":2.35,"country":"フランス","country_code":"FR","admin1":"イル＝ド＝フランス"}]});
        let p = parse_places(&v);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].detail, "東京都");
        assert_eq!(p[1].detail, "イル＝ド＝フランス フランス");
        assert!(parse_places(&json!({"generationtime_ms":0.1})).is_empty());
    }

    fn sample_area() -> Value {
        json!({
          "offices":{"130000":{"name":"東京都"},"140000":{"name":"神奈川県"}},
          "class10s":{"130010":{"parent":"130000"},"140010":{"parent":"140000"}},
          "class15s":{"130011":{"parent":"130010"},"140011":{"parent":"140010"}},
          "class20s":{
            "1310100":{"name":"千代田区","parent":"130011"},
            "1310200":{"name":"中央区","parent":"130011"},
            "1410000":{"name":"横浜市","parent":"140011"},
            "1413000":{"name":"川崎市","parent":"140011"},
            "2213001":{"name":"浜松市北部","parent":"x"},
            "2213002":{"name":"浜松市南部","parent":"x"}
          }
        })
    }

    #[test]
    fn area_matching() {
        let a = sample_area();
        assert_eq!(match_class20(&a, "13101"), vec!["1310100"]);
        // 横浜市西区 14103 → 横浜市、川崎市川崎区 14131 → 川崎市
        assert_eq!(match_class20(&a, "14103"), vec!["1410000"]);
        assert_eq!(match_class20(&a, "14131"), vec!["1413000"]);
        // 分割された区域
        assert_eq!(match_class20(&a, "22130"), vec!["2213001", "2213002"]);
        assert_eq!(office_of(&a, "1310200").as_deref(), Some("130000"));
        let info = area_info_from(&a, "1310100", None).unwrap();
        assert_eq!(info.office_name, "東京都");
        assert_eq!(info.options.len(), 2);
    }

    #[test]
    fn warnings_parse() {
        let v = json!([
          {"reportDatetime":"2026-06-06T13:30:00+09:00","warning":{"class10Items":[],"class20Items":[
            {"areaCode":"1310100","kinds":[{"code":"14","status":"発表"},{"code":"10","status":"解除"}]},
            {"areaCode":"1310200","kinds":[{"code":"03","status":"発表"}]}
          ]}},
          {"reportDatetime":"2026-06-06T14:00:00+09:00","warning":{"class20Items":[
            {"areaCode":"1310100","kinds":[{"code":"03","status":"継続"},{"code":"43","status":"発表"},{"code":"99","status":"発表"}]},
            {"areaCode":"1310100","kinds":[{"code":"14","status":"継続"},{"status":"発表警報・注意報はなし"}]}
          ]}}
        ]);
        let (items, t) = parse_warnings(&v, "1310100");
        let codes: Vec<&str> = items.iter().map(|i| i.code.as_str()).collect();
        assert_eq!(codes, vec!["43", "03", "14", "99"]);
        assert_eq!(items[3].name, "その他の警報・注意報");
        assert_eq!(t.as_deref(), Some("2026-06-06T14:00:00+09:00"));
        let (none, _) = parse_warnings(&json!([{"warning":{"class20Items":[{"areaCode":"1310100","kinds":[{"status":"発表警報・注意報はなし"}]}]}}]), "1310100");
        assert!(none.is_empty());
    }

    #[test]
    fn times_parse() {
        let v = json!([
          {"basetime":"20210227064500","validtime":"20210227064500","elements":["hrpns","hrpns_nd"]},
          {"basetime":"20210227064000","validtime":"20210227064000","elements":["hrpns","hrpns_nd"]},
          {"basetime":"bad","validtime":"20210227063500","elements":["hrpns"]},
          {"basetime":"20210227063000","validtime":"20210227063000","elements":["thns"]}
        ]);
        let f = parse_times(&v);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].validtime, "20210227064000");
    }

    #[test]
    fn enc() {
        assert_eq!(qenc("東京 a"), "%E6%9D%B1%E4%BA%AC%20a");
    }
}
