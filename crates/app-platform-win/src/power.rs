//! Prioridade e EcoQoS: o gravador não pode competir com o jogo pela CPU.
//!
//! - Ocioso (sem jogo): o processo inteiro em EcoQoS + prioridade baixa.
//! - Gravando: processo normal; threads auxiliares (salvar, miniatura,
//!   detecção, IPC, bandeja) em EcoQoS e prioridade abaixo do normal.
//!   A thread de vídeo fica normal para não perder quadros.

use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, ProcessPowerThrottling, SetPriorityClass,
    SetProcessInformation, SetThreadInformation, SetThreadPriority, ThreadPowerThrottling,
    BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
    THREAD_POWER_THROTTLING_STATE, THREAD_PRIORITY_BELOW_NORMAL,
};

/// `eco = true`: processo em EcoQoS e prioridade abaixo do normal (ocioso).
/// `eco = false`: deixa o Windows decidir e volta à prioridade normal (gravando).
pub fn set_process_eco(eco: bool) {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: if eco {
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED
        } else {
            0
        },
        StateMask: if eco {
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED
        } else {
            0
        },
    };
    unsafe {
        let p = GetCurrentProcess();
        if let Err(e) = SetProcessInformation(
            p,
            ProcessPowerThrottling,
            &state as *const _ as *const _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        ) {
            tracing::debug!("EcoQoS do processo indisponível: {e}");
        }
        let class = if eco {
            BELOW_NORMAL_PRIORITY_CLASS
        } else {
            NORMAL_PRIORITY_CLASS
        };
        let _ = SetPriorityClass(p, class);
    }
}

/// Marca a thread atual como trabalho de fundo (EcoQoS + prioridade baixa).
pub fn set_current_thread_background() {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
    };
    unsafe {
        let t = GetCurrentThread();
        let _ = SetThreadInformation(
            t,
            ThreadPowerThrottling,
            &state as *const _ as *const _,
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        );
        let _ = SetThreadPriority(t, THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Marca a thread atual como sensível a latência (nunca EcoQoS), mesmo se o
/// processo estiver em modo econômico.
pub fn set_current_thread_realtime_ish() {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    unsafe {
        let _ = SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            &state as *const _ as *const _,
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        );
    }
}
