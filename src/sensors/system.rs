use sysinfo::{Disks, Pid, ProcessesToUpdate, System};

use crate::{
    platform::PlatformAdapter,
    state::model::{
        CpuState, MachineState, MemoryState, ProcessSummary, StorageVolume, SystemState,
    },
};

pub struct SystemSampler {
    system: System,
    disks: Disks,
    platform: Box<dyn PlatformAdapter>,
}

impl SystemSampler {
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all();

        let disks = Disks::new_with_refreshed_list();
        Self {
            system,
            disks,
            platform: Box::<crate::platform::DefaultPlatformAdapter>::default(),
        }
    }

    pub fn sample(&mut self, max_processes: usize) -> SystemState {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        self.disks.refresh(true);

        let total_memory = self.system.total_memory();
        let used_memory = self.system.used_memory();
        let used_percent = if total_memory == 0 {
            0.0
        } else {
            used_memory as f32 / total_memory as f32 * 100.0
        };

        let processes = top_processes(&self.system, max_processes);
        let cpu_usage = self.system.global_cpu_usage();
        let per_core = self.system.cpus().iter().map(|cpu| cpu.cpu_usage()).collect();
        let storage = self
            .disks
            .list()
            .iter()
            .map(|disk| {
                let total = disk.total_space();
                let available = disk.available_space();
                let used_percent = if total == 0 {
                    0.0
                } else {
                    (total - available) as f32 / total as f32 * 100.0
                };

                StorageVolume {
                    name: disk.name().to_string_lossy().into_owned(),
                    mount_point: disk.mount_point().to_string_lossy().into_owned(),
                    total_bytes: total,
                    available_bytes: available,
                    used_percent,
                }
            })
            .collect();

        SystemState {
            collected_at: chrono::Utc::now(),
            machine: MachineState {
                hostname: System::host_name(),
                os_name: System::name(),
                kernel_version: System::kernel_version(),
                uptime_secs: System::uptime(),
            },
            cpu: CpuState {
                total_usage_percent: cpu_usage,
                per_core_usage_percent: per_core,
                temperature_celsius: None, // Will be supplemented by thermal_state if available
            },
            memory: MemoryState {
                total_bytes: total_memory,
                used_bytes: used_memory,
                swap_total_bytes: self.system.total_swap(),
                swap_used_bytes: self.system.used_swap(),
                used_percent,
            },
            storage,
            battery: self.platform.battery_state(),
            processes,
            alerts: self.collect_alerts(),
            container_runtime: None,
            network: None,
            peripheral_devices: None,
        }
    }

    fn collect_alerts(&self) -> Vec<crate::state::model::SystemAlert> {
        let mut alerts = Vec::new();
        if let Some(thermal) = self.platform.thermal_state() {
            if thermal != "Nominal" && thermal != "Fair" {
                alerts.push(crate::state::model::SystemAlert {
                    severity: crate::state::model::AlertSeverity::Warning,
                    code: "THERMAL_PRESSURE".into(),
                    message: format!("System thermal state is {}", thermal),
                });
            }
        }
        if let Some(gpu) = self.platform.gpu_usage_percent() {
            if gpu > 90.0 {
                alerts.push(crate::state::model::SystemAlert {
                    severity: crate::state::model::AlertSeverity::Warning,
                    code: "HIGH_GPU_USAGE".into(),
                    message: format!("GPU usage is at {:.1}%", gpu),
                });
            }
        }
        alerts
    }
}

fn top_processes(system: &System, max_processes: usize) -> Vec<ProcessSummary> {
    let mut processes = system
        .processes()
        .iter()
        .map(|(pid, process)| ProcessSummary {
            pid: pid.to_string(),
            name: process.name().to_string_lossy().into_owned(),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
            status: format!("{:?}", process.status()),
        })
        .collect::<Vec<_>>();

    processes.sort_by(|left, right| {
        right
            .cpu_percent
            .partial_cmp(&left.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
    });
    processes.truncate(max_processes);
    processes
}

#[allow(dead_code)]
fn _pid_to_string(pid: Pid) -> String {
    pid.to_string()
}
