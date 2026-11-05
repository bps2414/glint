//! Medidas do próprio processo (CPU e memória), para o modo de teste do player.

use std::time::Duration;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn filetime(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Tempo de CPU (usuário + kernel) gasto pelo processo até agora.
pub fn cpu_time() -> Option<Duration> {
    let (mut c, mut e, mut k, mut u) = Default::default();
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.ok()?;
    // FILETIME conta em unidades de 100 ns.
    Some(Duration::from_nanos((filetime(k) + filetime(u)) * 100))
}

/// Memória física em uso pelo processo (working set), em bytes.
pub fn working_set() -> Option<u64> {
    let mut pmc = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) }.ok()?;
    Some(pmc.WorkingSetSize as u64)
}
