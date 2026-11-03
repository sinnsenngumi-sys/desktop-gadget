//! CPU・GPU・RAMの使用量（§5）
//!
//! - CPU: 性能カウンター「Processor Information(_Total)\% Processor Utility」（Task Managerと同じ値）。
//!   取れない環境では「Processor(_Total)\% Processor Time」に切り替える。
//! - GPU: 性能カウンター「GPU Engine(*)\Utilization Percentage」。Task Managerと同じく、
//!   アダプターごとに「処理エンジン別の合計（全プロセス分）」を出し、その最大値を使用率とする。
//!   アダプター名はDXGIから取る。
//! - RAM: GlobalMemoryStatusEx。
//!
//! 使用率のカウンターは「前回の取得からの差」で計算するため、最初の1回は値が出ない。

use serde::Serialize;

/// 1つのGPUの使用率
#[derive(Serialize, Clone)]
pub struct GpuUsage {
    /// アダプターの識別子（LUID。"0000_0000D1A5" の形）。設定で固定するときに使う
    pub id: String,
    /// 短い名前（例: "Iris Xe"、"GTX 1650 Ti"）
    pub name: String,
    /// 元の名前（例: "Intel(R) Iris(R) Xe Graphics"）
    pub full_name: String,
    /// 0〜100。取れないときは None
    pub usage: Option<f64>,
}

#[derive(Serialize, Clone)]
pub struct Usage {
    /// 0〜100。最初の1回など、取れないときは None
    pub cpu: Option<f64>,
    /// 0〜100
    pub ram: Option<f64>,
    /// 使用中（GB、1024^3単位。Task Managerと同じ）
    pub ram_used_gb: f64,
    /// 搭載量（GB）
    pub ram_total_gb: f64,
    /// 見つかったGPU（使用率が取れないものも含む）
    pub gpus: Vec<GpuUsage>,
    /// 取得で起きた問題（画面の確認用。正常なら空）
    pub notes: Vec<String>,
}

/// GPU名を短くする: "Intel(R) Iris(R) Xe Graphics" → "Iris Xe"、"NVIDIA GeForce GTX 1650 Ti" → "GTX 1650 Ti"
pub fn short_gpu_name(full: &str) -> String {
    let mut s = full.to_string();
    for junk in ["(R)", "(r)", "(TM)", "(tm)", "®", "™"] {
        s = s.replace(junk, "");
    }
    let mut words: Vec<&str> = s.split_whitespace().collect();
    let drop_front = ["Intel", "NVIDIA", "AMD", "GeForce", "Microsoft"];
    while let Some(first) = words.first() {
        if drop_front.iter().any(|d| d.eq_ignore_ascii_case(first)) && words.len() > 1 {
            words.remove(0);
        } else {
            break;
        }
    }
    let drop_back = ["Graphics", "GPU", "Adapter"];
    while let Some(last) = words.last() {
        if drop_back.iter().any(|d| d.eq_ignore_ascii_case(last)) && words.len() > 1 {
            words.pop();
        } else {
            break;
        }
    }
    let out = words.join(" ");
    if out.is_empty() {
        full.trim().to_string()
    } else {
        out
    }
}

/// 性能カウンターのインスタンス名から、(LUIDの文字列, エンジンの識別子) を取り出す。
/// 例: "pid_1234_luid_0x00000000_0x0000D1A5_phys_0_eng_3_engtype_3D"
///   → ("00000000_0000D1A5", "0_3")
pub fn parse_engine_instance(name: &str) -> Option<(String, String)> {
    let t: Vec<&str> = name.split('_').collect();
    let li = t.iter().position(|x| x.eq_ignore_ascii_case("luid"))?;
    let hi = u32::from_str_radix(t.get(li + 1)?.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()?;
    let lo = u32::from_str_radix(t.get(li + 2)?.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()?;
    let phys = t
        .iter()
        .position(|x| x.eq_ignore_ascii_case("phys"))
        .and_then(|i| t.get(i + 1))
        .copied()
        .unwrap_or("0");
    let eng = t
        .iter()
        .position(|x| x.eq_ignore_ascii_case("eng"))
        .and_then(|i| t.get(i + 1))
        .copied()
        .unwrap_or("0");
    Some((luid_key(hi, lo), format!("{}_{}", phys, eng)))
}

pub fn luid_key(high: u32, low: u32) -> String {
    format!("{:08X}_{:08X}", high, low)
}

/// (LUID, エンジン, 使用率) の一覧から、アダプターごとの使用率（エンジン別合計の最大）を出す
pub fn aggregate_engines(items: &[(String, String, f64)]) -> std::collections::HashMap<String, f64> {
    use std::collections::HashMap;
    let mut per_engine: HashMap<(String, String), f64> = HashMap::new();
    for (luid, eng, v) in items {
        if v.is_finite() && *v > 0.0 {
            *per_engine.entry((luid.clone(), eng.clone())).or_insert(0.0) += v;
        } else {
            per_engine.entry((luid.clone(), eng.clone())).or_insert(0.0);
        }
    }
    let mut per_luid: HashMap<String, f64> = HashMap::new();
    for ((luid, _), v) in per_engine {
        let e = per_luid.entry(luid).or_insert(0.0);
        if v > *e {
            *e = v;
        }
    }
    for v in per_luid.values_mut() {
        *v = v.clamp(0.0, 100.0);
    }
    per_luid
}

// ---------------------------------------------------------------
// Windows の実装
// ---------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use super::*;
    use std::collections::HashMap;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    use windows::Win32::System::Performance::*;
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
    const ERROR_SUCCESS: u32 = 0;

    struct Adapter {
        id: String,
        full_name: String,
    }

    pub struct SysMon {
        query: PDH_HQUERY,
        cpu: Option<PDH_HCOUNTER>,
        gpu: Option<PDH_HCOUNTER>,
        adapters: Vec<Adapter>,
        adapters_err: Option<String>,
        init_notes: Vec<String>,
    }

    // PDHのハンドルは、Mutexで守ったうえで別スレッドから使ってよい
    unsafe impl Send for SysMon {}

    impl SysMon {
        pub fn new() -> Self {
            let mut notes = Vec::new();
            let mut query = PDH_HQUERY::default();
            let st = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
            if st != ERROR_SUCCESS {
                notes.push(format!("性能カウンターを開けませんでした（0x{:08X}）", st));
                let (adapters, adapters_err) = enum_adapters();
                return SysMon { query, cpu: None, gpu: None, adapters, adapters_err, init_notes: notes };
            }

            let add = |path: PCWSTR| -> Option<PDH_HCOUNTER> {
                let mut c = PDH_HCOUNTER::default();
                let st = unsafe { PdhAddEnglishCounterW(query, path, 0, &mut c) };
                if st == ERROR_SUCCESS {
                    Some(c)
                } else {
                    None
                }
            };

            let cpu = add(w!("\\Processor Information(_Total)\\% Processor Utility"))
                .or_else(|| add(w!("\\Processor(_Total)\\% Processor Time")));
            if cpu.is_none() {
                notes.push("CPUのカウンターを追加できませんでした".into());
            }
            let gpu = add(w!("\\GPU Engine(*)\\Utilization Percentage"));
            if gpu.is_none() {
                notes.push("GPUのカウンター（GPU Engine）がありません".into());
            }
            // 1回目の取得（差を取るための基準）
            unsafe {
                let _ = PdhCollectQueryData(query);
            }
            let (adapters, adapters_err) = enum_adapters();
            SysMon { query, cpu, gpu, adapters, adapters_err, init_notes: notes }
        }

        /// GPUの一覧を取り直す（設定画面を開いたときなど）
        pub fn refresh_adapters(&mut self) {
            let (a, e) = enum_adapters();
            self.adapters = a;
            self.adapters_err = e;
        }

        pub fn sample(&mut self) -> Usage {
            let mut notes = self.init_notes.clone();
            if let Some(e) = &self.adapters_err {
                notes.push(e.clone());
            }
            let collected = !self.query.is_invalid() && unsafe { PdhCollectQueryData(self.query) } == ERROR_SUCCESS;

            // CPU
            let cpu = match (collected, self.cpu) {
                (true, Some(c)) => {
                    let mut v = PDH_FMT_COUNTERVALUE::default();
                    let st = unsafe { PdhGetFormattedCounterValue(c, PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100), None, &mut v) };
                    if st == ERROR_SUCCESS && (v.CStatus == PDH_CSTATUS_VALID_DATA || v.CStatus == PDH_CSTATUS_NEW_DATA) {
                        let d = unsafe { v.Anonymous.doubleValue };
                        if d.is_finite() {
                            Some(d.clamp(0.0, 100.0))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            // GPU
            let per_luid: Option<HashMap<String, f64>> = match (collected, self.gpu) {
                (true, Some(c)) => read_array(c).map(|items| {
                    let parsed: Vec<(String, String, f64)> = items
                        .into_iter()
                        .filter_map(|(name, v)| parse_engine_instance(&name).map(|(l, e)| (l, e, v)))
                        .collect();
                    aggregate_engines(&parsed)
                }),
                _ => None,
            };

            let mut gpus: Vec<GpuUsage> = Vec::new();
            let any_match = per_luid
                .as_ref()
                .map(|m| self.adapters.iter().any(|a| m.contains_key(&a.id)))
                .unwrap_or(false);
            for a in &self.adapters {
                // カウンターにまだ出ていないアダプター（外付けGPUが休止中など）は 0%
                let usage = per_luid.as_ref().map(|m| m.get(&a.id).copied().unwrap_or(0.0));
                gpus.push(GpuUsage { id: a.id.clone(), name: short_gpu_name(&a.full_name), full_name: a.full_name.clone(), usage });
            }
            // DXGIの名前と1つも対応しないときは、名前なしで出す（取り違えより「出ない」を避ける）
            if let Some(m) = &per_luid {
                if !any_match {
                    let mut keys: Vec<&String> = m.keys().collect();
                    keys.sort();
                    for (i, k) in keys.into_iter().enumerate() {
                        let n = format!("GPU {}", i);
                        gpus.push(GpuUsage { id: k.clone(), name: n.clone(), full_name: n, usage: m.get(k).copied() });
                    }
                    if !self.adapters.is_empty() {
                        notes.push("GPUの名前とカウンターを対応づけられませんでした".into());
                    }
                }
            }

            // RAM
            let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            let (ram, used, total) = match unsafe { GlobalMemoryStatusEx(&mut ms) } {
                Ok(()) if ms.ullTotalPhys > 0 => {
                    let total = ms.ullTotalPhys as f64;
                    let used = (ms.ullTotalPhys - ms.ullAvailPhys) as f64;
                    let g = 1024.0 * 1024.0 * 1024.0;
                    (Some((used / total * 100.0).clamp(0.0, 100.0)), used / g, total / g)
                }
                _ => {
                    notes.push("メモリの情報を取得できませんでした".into());
                    (None, 0.0, 0.0)
                }
            };

            Usage { cpu, ram, ram_used_gb: used, ram_total_gb: total, gpus, notes }
        }
    }

    impl Drop for SysMon {
        fn drop(&mut self) {
            if !self.query.is_invalid() {
                unsafe {
                    let _ = PdhCloseQuery(self.query);
                }
            }
        }
    }

    /// ワイルドカードのカウンターを、(インスタンス名, 値) の一覧で読む
    fn read_array(c: PDH_HCOUNTER) -> Option<Vec<(String, f64)>> {
        let fmt = PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100);
        // 読む間にプロセスが増えると大きさが変わるので、数回まで取り直す
        for _ in 0..4 {
            let mut size: u32 = 0;
            let mut count: u32 = 0;
            let st = unsafe { PdhGetFormattedCounterArrayW(c, fmt, &mut size, &mut count, None) };
            if st != PDH_MORE_DATA {
                // インスタンスが1つもない（GPUが休止中など）
                return if st == ERROR_SUCCESS { Some(Vec::new()) } else { None };
            }
            // 余裕を持って確保する。8バイト境界に合わせるため u64 の配列を使う
            let bytes = size as usize + 4096;
            let mut buf: Vec<u64> = vec![0; bytes.div_ceil(8)];
            let mut size2 = (buf.len() * 8) as u32;
            let mut count2: u32 = 0;
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            let st = unsafe { PdhGetFormattedCounterArrayW(c, fmt, &mut size2, &mut count2, Some(items)) };
            if st == PDH_MORE_DATA {
                continue;
            }
            if st != ERROR_SUCCESS {
                return None;
            }
            let mut out = Vec::with_capacity(count2 as usize);
            for i in 0..count2 as usize {
                let it = unsafe { &*items.add(i) };
                if it.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA && it.FmtValue.CStatus != PDH_CSTATUS_NEW_DATA {
                    continue;
                }
                let name = unsafe { it.szName.to_string() }.unwrap_or_default();
                let v = unsafe { it.FmtValue.Anonymous.doubleValue };
                out.push((name, v));
            }
            return Some(out);
        }
        None
    }

    /// DXGIでGPUの一覧（ソフトウェアのアダプターを除く）
    fn enum_adapters() -> (Vec<Adapter>, Option<String>) {
        let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
            Ok(f) => f,
            Err(e) => return (Vec::new(), Some(format!("GPUの一覧を取得できませんでした: {}", e))),
        };
        let mut out: Vec<Adapter> = Vec::new();
        let mut i = 0u32;
        while let Ok(a) = unsafe { factory.EnumAdapters1(i) } {
            i += 1;
            let Ok(d) = (unsafe { a.GetDesc1() }) else { continue };
            if d.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let len = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
            let full_name = String::from_utf16_lossy(&d.Description[..len]).trim().to_string();
            let id = luid_key(d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart);
            // 同じアダプターが出力ごとに重複して出ることがあるので除く
            if !out.iter().any(|x| x.id == id) {
                out.push(Adapter { id, full_name });
            }
            if i > 16 {
                break;
            }
        }
        (out, None)
    }
}

// ---------------------------------------------------------------
// Windows 以外（画面の確認用。値は出さない）
// ---------------------------------------------------------------

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub struct SysMon;
    impl SysMon {
        pub fn new() -> Self {
            SysMon
        }
        pub fn refresh_adapters(&mut self) {}
        pub fn sample(&mut self) -> Usage {
            Usage { cpu: None, ram: None, ram_used_gb: 0.0, ram_total_gb: 0.0, gpus: Vec::new(), notes: vec!["Windows専用の機能です".into()] }
        }
    }
}

pub use imp::SysMon;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names() {
        assert_eq!(short_gpu_name("Intel(R) Iris(R) Xe Graphics"), "Iris Xe");
        assert_eq!(short_gpu_name("NVIDIA GeForce GTX 1650 Ti"), "GTX 1650 Ti");
        assert_eq!(short_gpu_name("AMD Radeon(TM) Graphics"), "Radeon");
        assert_eq!(short_gpu_name("Intel(R) UHD Graphics 620"), "UHD Graphics 620");
    }

    #[test]
    fn parse_instance() {
        let (l, e) = parse_engine_instance("pid_1234_luid_0x00000000_0x0000D1A5_phys_0_eng_3_engtype_3D").unwrap();
        assert_eq!(l, "00000000_0000D1A5");
        assert_eq!(e, "0_3");
        assert!(parse_engine_instance("garbage").is_none());
    }

    #[test]
    fn aggregate_like_task_manager() {
        // 同じエンジン(0_0)を2つのプロセスが使う → 合計30。別エンジン(0_1)は20 → アダプターの値は30
        let items = vec![
            ("A".to_string(), "0_0".to_string(), 10.0),
            ("A".to_string(), "0_0".to_string(), 20.0),
            ("A".to_string(), "0_1".to_string(), 20.0),
            ("B".to_string(), "0_0".to_string(), 0.0),
            ("C".to_string(), "0_0".to_string(), 80.0),
            ("C".to_string(), "0_0".to_string(), 50.0),
        ];
        let m = aggregate_engines(&items);
        assert_eq!(m["A"], 30.0);
        assert_eq!(m["B"], 0.0);
        assert_eq!(m["C"], 100.0);
    }
}
