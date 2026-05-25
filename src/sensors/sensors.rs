// sensors.rs
use sysinfo::{System, CpuExt, DiskExt, NetworkExt, ProcessExt};
use serde::Serialize;

#[derive(Serialize)]
pub struct SystemState {
    pub cpu_usage: f32,
    pub mem_used_mb: u64,
    pub active_processes: usize,
    // Add other fields from your JSON requirement here
}

pub fn capture_state() -> SystemState {
    let mut sys = System::new_all();
    sys.refresh_all();

    SystemState {
        cpu_usage: sys.global_cpu_info().cpu_usage(),
        mem_used_mb: sys.used_memory() / 1024 / 1024,
        active_processes: sys.processes().len(),
    }
}