use crate::state::model::BatteryState;

#[cfg(target_os = "macos")]
pub mod macos;

pub trait PlatformAdapter: Send + Sync {
    fn battery_state(&self) -> Option<BatteryState>;
    fn gpu_usage_percent(&self) -> Option<f32>;
    fn thermal_state(&self) -> Option<String>;
    #[allow(dead_code)]
    fn accessibility_available(&self) -> bool;
    #[allow(dead_code)]
    fn startup_registration_supported(&self) -> bool;
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub struct DefaultPlatformAdapter;

#[cfg(not(target_os = "macos"))]
impl PlatformAdapter for DefaultPlatformAdapter {
    fn battery_state(&self) -> Option<BatteryState> {
        None
    }

    fn gpu_usage_percent(&self) -> Option<f32> {
        None
    }

    fn thermal_state(&self) -> Option<String> {
        None
    }

    fn accessibility_available(&self) -> bool {
        false
    }

    fn startup_registration_supported(&self) -> bool {
        cfg!(target_os = "linux") || cfg!(target_os = "macos") || cfg!(target_os = "windows")
    }
}

#[cfg(target_os = "macos")]
pub type DefaultPlatformAdapter = macos::MacosPlatformAdapter;

