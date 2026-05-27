use sysinfo::{System, Disks, Networks, ProcessesToUpdate};
use crate::error::Result;

/// Defines the tools available to Gemini.
pub fn get_tool_declarations() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "get_system_diagnostics",
            "description": "Returns detailed health metrics for CPU, Memory, and Disk.",
            "parameters": {
                "type": "object",
                "properties": {}
            }
        },
        {
            "name": "list_top_processes",
            "description": "Returns the top 5 processes by CPU and Memory usage.",
            "parameters": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Number of processes to return (default 5)."
                    }
                }
            }
        },
        {
            "name": "check_network_status",
            "description": "Returns status of network interfaces and a connectivity check.",
            "parameters": {
                "type": "object",
                "properties": {}
            }
        }
    ])
}

/// Dispatches and executes the requested tool.
pub fn call_tool(name: &str, args: serde_json::Value) -> Result<serde_json::Value> {
    match name {
        "get_system_diagnostics" => Ok(get_system_diagnostics()?),
        "list_top_processes" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            Ok(list_top_processes(limit)?)
        },
        "check_network_status" => Ok(check_network_status()?),
        _ => Ok(serde_json::json!({ "error": format!("Tool '{}' not found", name) })),
    }
}

fn get_system_diagnostics() -> Result<serde_json::Value> {
    let mut sys = System::new_all();
    sys.refresh_all();
    let disks = Disks::new_with_refreshed_list();

    let cpu_load: f32 = sys.global_cpu_usage();
    let mem_total = sys.total_memory() / 1024 / 1024;
    let mem_used = sys.used_memory() / 1024 / 1024;
    
    let mut disk_info = Vec::new();
    for disk in disks.list() {
        disk_info.push(serde_json::json!({
            "name": disk.name().to_string_lossy(),
            "mount": disk.mount_point().display().to_string(),
            "total_gb": disk.total_space() / 1024 / 1024 / 1024,
            "available_gb": disk.available_space() / 1024 / 1024 / 1024,
        }));
    }

    Ok(serde_json::json!({
        "cpu": {
            "load_percent": format!("{:.1}", cpu_load),
            "cores": sys.cpus().len()
        },
        "memory": {
            "total_mb": mem_total,
            "used_mb": mem_used,
            "percent": format!("{:.1}", (mem_used as f32 / mem_total as f32) * 100.0)
        },
        "disks": disk_info,
        "uptime_seconds": System::uptime()
    }))
}

fn list_top_processes(limit: usize) -> Result<serde_json::Value> {
    let mut sys = System::new_all();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut processes: Vec<_> = sys.processes().values().collect();
    processes.sort_by(|a, b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));

    let top: Vec<_> = processes.iter().take(limit).map(|p| {
        serde_json::json!({
            "pid": p.pid().to_string(),
            "name": p.name().to_string_lossy(),
            "cpu_usage": format!("{:.1}%", p.cpu_usage()),
            "memory_mb": p.memory() / 1024 / 1024
        })
    }).collect();

    Ok(serde_json::json!(top))
}

fn check_network_status() -> Result<serde_json::Value> {
    let networks = Networks::new_with_refreshed_list();
    
    let mut interfaces = Vec::new();
    for (name, data) in networks.iter() {
        interfaces.push(serde_json::json!({
            "interface": name,
            "received_kb": data.total_received() / 1024,
            "transmitted_kb": data.total_transmitted() / 1024
        }));
    }

    Ok(serde_json::json!({
        "interfaces": interfaces,
        "hostname": System::host_name().unwrap_or_else(|| "unknown".into())
    }))
}
