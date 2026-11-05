//! Relógio comum: QueryPerformanceCounter em microssegundos.
//! Os timestamps do WASAPI (`u64QPCPosition`, em 100 ns) estão no mesmo domínio.

use std::sync::OnceLock;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

fn freq() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        unsafe { QueryPerformanceFrequency(&mut f).ok() };
        f.max(1)
    })
}

/// Agora, em µs desde o boot (domínio QPC).
pub fn now_us() -> i64 {
    let mut c = 0i64;
    unsafe { QueryPerformanceCounter(&mut c).ok() };
    let f = freq();
    (c / f) * 1_000_000 + (c % f) * 1_000_000 / f
}

/// Converte unidades de 100 ns do QPC (WASAPI/WGC) para µs.
pub fn hns_to_us(hns: u64) -> i64 {
    (hns / 10) as i64
}
