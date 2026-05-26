use std::{fs, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::error::{InternalVoiceError, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    pub service: ServiceConfig,
    pub secrets: SecretConfig,
    pub limits: LimitConfig,
    pub narration: NarrationConfig,
    pub policy: PolicyConfig,
    #[serde(default)]
    pub alerts: AlertPreferences,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlertPreferences {
    pub frequency: NotificationFrequency,
    pub setup_completed: bool,
}

impl Default for AlertPreferences {
    fn default() -> Self {
        Self {
            frequency: NotificationFrequency::None,
            setup_completed: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NotificationFrequency {
    Minute,
    Hourly,
    Daily,
    None,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceConfig {
    pub sample_interval_secs: u64,
    pub gemini_timeout_secs: u64,
    pub gemini_model: String,

    #[serde(default = "default_gemini_live_model")]
    pub gemini_live_model: String,

    #[serde(default)]
    pub gemini_api_key: String,

    pub debounce_millis: u64,
    pub max_prompt_tokens_hint: u32,
    pub cache_ttl_secs: u64,
    pub max_log_files: usize,

    #[serde(default = "default_log_dir")]
    pub log_dir: String,
    pub max_log_file_size_mb: usize,

    /// Override the Gemini Live WebSocket endpoint when the default path is not available
    /// for your account/model.
    #[serde(default = "default_gemini_live_ws_url")]
    pub gemini_live_ws_url: String,
}

fn default_gemini_live_model() -> String {
    "gemini-2.0-flash-exp".to_string()
}

fn default_gemini_live_ws_url() -> String {
    "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1alpha.GenerativeService/BiDiGenerateContent?key={key}".to_string()
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecretConfig {
    pub env_var: String,
    pub keyring_service: String,
    pub keyring_account: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LimitConfig {
    pub requests_per_minute: u32,
    pub max_cpu_percent: f32,
    pub max_memory_mb: u64,
    pub breaker_failure_threshold: u32,
    pub breaker_reset_secs: u64,
    #[serde(default = "default_system_telemetry_interval_secs")]
    pub system_telemetry_interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NarrationConfig {
    pub min_announcement_interval_secs: u64,
    pub voice_enabled: bool,
    pub max_processes: usize,
    pub style: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicyConfig {
    pub notify_on_battery_below: f32,
    pub notify_on_disk_below: f32,
    pub notify_on_cpu_above: f32,
    pub notify_on_memory_above: f32,
    pub system_telemetry: Option<SystemTelemetryPolicy>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SystemTelemetryPolicy {
    pub host_metadata: HostMetadataPolicy,
    pub hardware_resources: HardwareResourcesPolicy,
    pub running_processes: Vec<ProcessPolicy>,
    pub container_runtime: ContainerRuntimePolicy,
    pub network: NetworkPolicy,
    pub peripheral_devices: PeripheralDevicesPolicy,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HostMetadataPolicy {
    pub hostname: String,
    pub os_version: String,
    pub kernel_version: String,
    pub uptime_seconds: String,
    pub cpu_model: String,
    pub total_memory_bytes: String,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HardwareResourcesPolicy {
    pub cpu_usage_percent: String,
    pub memory_usage_percent: String,
    pub storage: Vec<StoragePolicy>,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StoragePolicy {
    pub mount_point: String,
    pub filesystem: String,
    pub total_bytes: String,
    pub used_bytes: String,
    pub read_iops: String,
    pub write_iops: String,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessPolicy {
    pub pid: String,
    pub name: String,
    pub user: String,
    pub cpu_percent: String,
    pub memory_percent: String,
    pub command_line: String,
    pub start_time: String,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContainerRuntimePolicy {
    pub engine: String,
    pub containers: Vec<ContainerPolicy>,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContainerPolicy {
    pub container_id: String,
    pub image: String,
    pub status: String,
    pub created: String,
    pub ports: Vec<String>,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkPolicy {
    pub active_connections: Vec<ConnectionPolicy>,
    pub interfaces: Vec<InterfacePolicy>,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionPolicy {
    pub protocol: String,
    pub local_address: String,
    pub local_port: String,
    pub remote_address: String,
    pub remote_port: String,
    pub state: String,
    pub process_name: String,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InterfacePolicy {
    pub name: String,
    pub mac_address: String,
    pub ip_addresses: Vec<String>,
    pub bytes_sent: String,
    pub bytes_received: String,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeripheralDevicesPolicy {
    pub bluetooth_connected: Vec<BluetoothPolicy>,
    pub usb_devices: Vec<UsbPolicy>,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BluetoothPolicy {
    pub device_name: String,
    pub mac_address: String,
    pub class: String,
    pub connected_at: String,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UsbPolicy {
    pub vendor_id: String,
    pub product_id: String,
    pub serial: String,
}

fn default_log_dir() -> String {
    "logs".to_string()
}

fn default_system_telemetry_interval_secs() -> u64 {
    60
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        let content = fs::read_to_string(&path).map_err(|err| {
            InternalVoiceError::Config(format!("failed reading config {}: {err}", path.display()))
        })?;

        toml::from_str(&content)
            .map_err(|err| InternalVoiceError::Config(format!("invalid config: {err}")))
    }

    pub fn sample_interval(&self) -> Duration {
        Duration::from_secs(self.service.sample_interval_secs)
    }

    pub fn gemini_timeout(&self) -> Duration {
        Duration::from_secs(self.service.gemini_timeout_secs)
    }

    pub fn breaker_reset(&self) -> Duration {
        Duration::from_secs(self.limits.breaker_reset_secs)
    }

    pub fn announcement_interval(&self) -> Duration {
        Duration::from_secs(self.narration.min_announcement_interval_secs)
    }

    pub fn system_telemetry_interval(&self) -> Duration {
        Duration::from_secs(self.limits.system_telemetry_interval_secs)
    }

    pub fn log_dir(&self) -> PathBuf {
        PathBuf::from(&self.service.log_dir)
    }

    #[allow(dead_code)]
    pub fn max_log_file_size(&self) -> u64 {
        (self.service.max_log_file_size_mb as u64) * 1024 * 1024
    }

     pub fn max_log_files(&self) -> usize {
        self.service.max_log_files
    }

    pub fn debounce_interval(&self) -> Duration {
        Duration::from_millis(self.service.debounce_millis)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        let content = toml::to_string_pretty(self).map_err(|err| {
            InternalVoiceError::Config(format!("failed to serialize config: {err}"))
        })?;
        fs::write(&path, content).map_err(|err| {
            InternalVoiceError::Config(format!("failed writing config {}: {err}", path.display()))
        })?;
        Ok(())
    }
}

pub fn config_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("INTERNALVOICE_CONFIG") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        tracing::warn!(
            "INTERNALVOICE_CONFIG points to {}, but the file does not exist; falling back",
            path.display()
        );
    }

    // Check current directory first for convenience
    let local = PathBuf::from("config.toml");
    if local.exists() {
        return Ok(local);
    }

    let example = PathBuf::from("config/InternalVoice.example.toml");
    if example.exists() {
        return Ok(example);
    }

    let mut base = dirs::config_dir()
        .ok_or_else(|| InternalVoiceError::Config("unable to resolve config directory".into()))?;
    base.push("InternalVoice");
    base.push("config.toml");
    Ok(base)
}

pub fn data_dir() -> Result<PathBuf> {
    let mut base = dirs::data_local_dir()
        .ok_or_else(|| InternalVoiceError::Config("unable to resolve data directory".into()))?;
    base.push("InternalVoice");
    Ok(base)
}
