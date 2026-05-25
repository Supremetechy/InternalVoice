use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemState {
    pub collected_at: DateTime<Utc>,
    pub machine: MachineState,
    pub cpu: CpuState,
    pub memory: MemoryState,
    pub storage: Vec<StorageVolume>,
    pub battery: Option<BatteryState>,
    pub processes: Vec<ProcessSummary>,
    pub alerts: Vec<SystemAlert>,
    pub container_runtime: Option<ContainerRuntimeState>,
    pub network: Option<NetworkState>,
    pub peripheral_devices: Option<PeripheralDevicesState>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRuntimeState {
    pub engine: String,
    pub containers: Vec<ContainerState>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerState {
    pub container_id: String,
    pub image: String,
    pub status: String,
    pub created: DateTime<Utc>,
    pub ports: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkState {
    pub active_connections: Vec<NetworkConnection>,
    pub interfaces: Vec<NetworkInterface>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConnection {
    pub protocol: String,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub state: String,
    pub process_name: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub mac_address: Option<String>,
    pub ip_addresses: Vec<String>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeripheralDevicesState {
    pub bluetooth_connected: Vec<BluetoothDevice>,
    pub usb_devices: Vec<UsbDevice>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothDevice {
    pub device_name: String,
    pub mac_address: String,
    pub class: String,
    pub connected_at: DateTime<Utc>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbDevice {
    pub vendor_id: String,
    pub product_id: String,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineState {
    pub hostname: Option<String>,
    pub os_name: Option<String>,
    pub kernel_version: Option<String>,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuState {
    pub total_usage_percent: f32,
    pub per_core_usage_percent: Vec<f32>,
    pub temperature_celsius: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryState {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub used_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageVolume {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryState {
    pub percent: f32,
    pub on_ac_power: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSummary {
    pub pid: String,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAlert {
    pub severity: AlertSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

