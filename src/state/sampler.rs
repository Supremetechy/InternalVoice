use std::time::Instant;
use chrono::Utc;
use tracing::debug;

use crate::{
    config::AppConfig,
    sensors::system::SystemSampler,
    state::model::{AlertSeverity, SystemAlert, SystemState},
};

pub struct SystemStateService {
    sampler: SystemSampler,
    last_telemetry_sample: Option<Instant>,
}

impl SystemStateService {
    pub fn new() -> Self {
        Self {
            sampler: SystemSampler::new(),
            last_telemetry_sample: None,
        }
    }

    pub fn sample(&mut self, config: &AppConfig) -> SystemState {
        let now = Instant::now();
        let should_collect_full = self.last_telemetry_sample
            .map(|last| last.elapsed() >= config.system_telemetry_interval())
            .unwrap_or(true);

        if should_collect_full {
            debug!(interval = ?config.system_telemetry_interval(), "Collecting full system telemetry");
            self.last_telemetry_sample = Some(now);
        }

        let mut state = self.sampler.sample(config.narration.max_processes);
        state.collected_at = Utc::now();
        let mut alerts = evaluate_alerts(&state, config);
        state.alerts.append(&mut alerts);
        state
    }
}

fn evaluate_alerts(state: &SystemState, config: &AppConfig) -> Vec<SystemAlert> {
    let mut alerts = Vec::new();

    if state.cpu.total_usage_percent >= config.policy.notify_on_cpu_above {
        alerts.push(SystemAlert {
            severity: AlertSeverity::Warning,
            code: "cpu.high".into(),
            message: format!("CPU usage is {:.1}%", state.cpu.total_usage_percent),
        });
    }

    if state.memory.used_percent >= config.policy.notify_on_memory_above {
        alerts.push(SystemAlert {
            severity: AlertSeverity::Warning,
            code: "memory.high".into(),
            message: format!("Memory usage is {:.1}%", state.memory.used_percent),
        });
    }

    for volume in &state.storage {
        let free_percent = 100.0 - volume.used_percent;
        if free_percent <= config.policy.notify_on_disk_below {
            alerts.push(SystemAlert {
                severity: AlertSeverity::Warning,
                code: "disk.low".into(),
                message: format!(
                    "{} has only {:.1}% free space remaining",
                    volume.name, free_percent
                ),
            });
        }
    }

    if let Some(battery) = &state.battery {
        if !battery.on_ac_power && battery.percent <= config.policy.notify_on_battery_below {
            alerts.push(SystemAlert {
                severity: AlertSeverity::Critical,
                code: "battery.low".into(),
                message: format!("Battery is at {:.1}%", battery.percent),
            });
        }
    }

    alerts
}

