use std::process::Command;
use sysinfo::{Disks, System};

/// Hardware profile captured from the host system.
pub struct HardwareProfile {
    pub os: String,
    pub cpu_model: String,
    pub gpu_model: String,
    pub vram_gb: f64,
    pub ram_gb: f64,
    pub free_disk_gb: f64,
    pub disk_path: String,
    pub is_apple_silicon: bool,
    pub eim_gb: f64,
    pub tier: u8,
}

/// Runs OS-specific hardware detection and returns a complete profile.
pub fn detect_hardware() -> HardwareProfile {
    let mut sys = System::new_all();
    sys.refresh_all();

    let ram_gb = sys.total_memory() as f64 / 1_073_741_824.0;
    let cpu_model = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_else(|| "Unknown CPU".to_string());

    let os = {
        let name = System::name().unwrap_or_else(|| "Unknown".to_string());
        let ver = System::os_version().unwrap_or_default();
        if ver.is_empty() {
            name
        } else {
            format!("{} {}", name, ver)
        }
    };

    // Apple Silicon: brand string contains "Apple M" OR arm64 on macOS
    let is_apple_silicon = cpu_model.to_lowercase().contains("apple m")
        || (os.to_lowercase().contains("macos")
            && System::cpu_arch().to_lowercase().contains("arm"));

    let (gpu_model, vram_gb) = detect_gpu_info(&os, is_apple_silicon, ram_gb);
    let (free_disk_gb, disk_path) = detect_disk_info();
    let eim_gb = compute_eim(vram_gb, ram_gb, is_apple_silicon);
    let tier = map_eim_to_tier(eim_gb);

    HardwareProfile {
        os,
        cpu_model,
        gpu_model,
        vram_gb,
        ram_gb,
        free_disk_gb,
        disk_path,
        is_apple_silicon,
        eim_gb,
        tier,
    }
}

fn detect_gpu_info(os: &str, is_apple_silicon: bool, ram_gb: f64) -> (String, f64) {
    // Apple Silicon: unified memory pool, no separate VRAM
    if is_apple_silicon {
        let chip = detect_apple_chip_model();
        return (format!("{} (unified)", chip), ram_gb);
    }

    // NVIDIA: nvidia-smi is authoritative and handles the 32-bit overflow issue
    if let Ok(out) = Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().next() {
                let parts: Vec<&str> = line.splitn(2, ',').collect();
                if parts.len() == 2 {
                    let name = parts[0].trim().to_string();
                    let vram_mib: f64 = parts[1]
                        .trim()
                        .replace(" MiB", "")
                        .replace("MiB", "")
                        .parse()
                        .unwrap_or(0.0);
                    if vram_mib > 0.0 {
                        return (name, vram_mib / 1024.0);
                    }
                }
            }
        }
    }

    // macOS discrete GPU via system_profiler
    if os.to_lowercase().contains("macos") {
        if let Some((name, vram)) = probe_macos_discrete_gpu() {
            return (name, vram);
        }
    }

    // AMD: rocm-smi
    if let Ok(out) = Command::new("rocm-smi").args(["--showmeminfo", "vram"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if line.to_lowercase().contains("vram total memory") {
                    if let Some(mb_str) = line.split(':').last() {
                        let mb: f64 = mb_str
                            .trim()
                            .split_whitespace()
                            .next()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0.0);
                        if mb > 0.0 {
                            return ("AMD GPU (ROCm)".to_string(), mb / 1024.0);
                        }
                    }
                }
            }
        }
    }

    ("No discrete GPU (CPU inference)".to_string(), 0.0)
}

fn detect_apple_chip_model() -> String {
    // sysctl returns the full brand string, e.g. "Apple M4 Pro"
    if let Ok(out) = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    // Fallback: system_profiler hardware data lists "Chip:" on Apple Silicon
    if let Ok(out) = Command::new("system_profiler")
        .arg("SPHardwareDataType")
        .output()
    {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let t = line.trim();
                if t.starts_with("Chip:") {
                    return t.trim_start_matches("Chip:").trim().to_string();
                }
            }
        }
    }
    "Apple Silicon".to_string()
}

fn probe_macos_discrete_gpu() -> Option<(String, f64)> {
    let out = Command::new("system_profiler")
        .arg("SPDisplaysDataType")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut name = String::new();
    let mut vram_gb = 0.0f64;

    for line in s.lines() {
        let t = line.trim();
        if t.starts_with("Chipset Model:") {
            name = t.trim_start_matches("Chipset Model:").trim().to_string();
        }
        for key in &[
            "VRAM (Total):",
            "VRAM (Dynamic, Max):",
            "VRAM (Static):",
        ] {
            if t.starts_with(key) {
                let val = t.trim_start_matches(key).trim();
                if val.contains("GB") {
                    vram_gb = val.replace("GB", "").trim().parse().unwrap_or(0.0);
                } else if val.contains("MB") {
                    let mb: f64 = val.replace("MB", "").trim().parse().unwrap_or(0.0);
                    vram_gb = mb / 1024.0;
                }
                break;
            }
        }
    }

    if vram_gb > 0.0 {
        Some((name, vram_gb))
    } else {
        None
    }
}

fn detect_disk_info() -> (f64, String) {
    let disks = Disks::new_with_refreshed_list();
    for disk in disks.list() {
        let mp = disk.mount_point().to_string_lossy();
        if mp == "/" || mp == "C:\\" || mp == "C:/" {
            return (
                disk.available_space() as f64 / 1_073_741_824.0,
                mp.into_owned(),
            );
        }
    }
    if let Some(d) = disks.list().first() {
        return (
            d.available_space() as f64 / 1_073_741_824.0,
            d.mount_point().to_string_lossy().into_owned(),
        );
    }
    (0.0, "unknown".to_string())
}

/// EIM formula from the LOCAL LLM RECOMMENDER skill spec.
/// Apple Silicon: 75% of unified RAM (macOS default GPU allocation cap).
/// Discrete GPU: VRAM only.
/// CPU-only / iGPU: 50% of system RAM.
pub fn compute_eim(vram_gb: f64, ram_gb: f64, is_apple_silicon: bool) -> f64 {
    if is_apple_silicon {
        ram_gb * 0.75
    } else if vram_gb > 0.0 {
        vram_gb
    } else {
        ram_gb * 0.5
    }
}

/// Maps EIM (GB) to a hardware tier T0–T9.
/// Gaps in the table fall into the lower tier.
pub fn map_eim_to_tier(eim_gb: f64) -> u8 {
    if eim_gb < 6.0 {
        0
    } else if eim_gb < 10.0 {
        1
    } else if eim_gb < 16.0 {
        2
    } else if eim_gb < 22.0 {
        3
    } else if eim_gb < 30.0 {
        4
    } else if eim_gb < 45.0 {
        5
    } else if eim_gb < 70.0 {
        6
    } else if eim_gb < 100.0 {
        7
    } else if eim_gb < 200.0 {
        8
    } else {
        9
    }
}

struct TierModel {
    name: &'static str,
    quant: &'static str,
    aa_score: Option<u32>,
    size_gb: f64,
    speed: &'static str,
    why: &'static str,
    install: &'static str,
}

/// Best model pick per tier, ranked by Artificial Analysis Intelligence Index (snapshot 11-05-2026).
/// Qwen3.6 picks use llama-cli + Unsloth GGUFs — Ollama does not yet support Qwen3.6 vision files.
fn best_for_tier(tier: u8) -> TierModel {
    match tier {
        0 => TierModel {
            name: "Qwen3 4B",
            quant: "Q4_K_M",
            aa_score: None,
            size_gb: 2.5,
            speed: "Fast (>30 tok/s)",
            why: "Lightweight 4B; best choice for sub-6 GB EIM hardware",
            install: "ollama run qwen3:4b",
        },
        1 => TierModel {
            name: "Qwen3.5 9B",
            quant: "Q4_K_M",
            aa_score: Some(27),
            size_gb: 5.4,
            speed: "Fast (>30 tok/s)",
            why: "Highest AA score in the 6–9 GB EIM tier; fits cleanly",
            install: "ollama run qwen3.5:9b",
        },
        2 => TierModel {
            name: "gpt-oss 20B",
            quant: "MXFP4",
            aa_score: Some(24),
            size_gb: 11.0,
            speed: "Usable (10–30 tok/s)",
            why: "OpenAI open-weight 20B at efficient MXFP4; strong quality for 10–14 GB EIM",
            install: "ollama run gpt-oss:20b",
        },
        3 => TierModel {
            name: "Qwen3.6-27B",
            quant: "Q4_K_M",
            aa_score: Some(37),
            size_gb: 16.2,
            speed: "Usable (10–30 tok/s)",
            why: "Highest AA score for 16–20 GB EIM; use llama-cli (Ollama not yet supported)",
            install: "llama-cli -hf unsloth/Qwen3.6-27B-GGUF:UD-Q4_K_XL",
        },
        4 => TierModel {
            name: "Qwen3.6-35B-A3B",
            quant: "Q4_K_M",
            aa_score: Some(43),
            size_gb: 21.0,
            speed: "Usable (10–30 tok/s)",
            why: "MoE 35B — highest AA score across all tiers; runs at 3B active-param speed",
            install: "llama-cli -hf unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q4_K_XL",
        },
        5 => TierModel {
            name: "Qwen3.6-35B-A3B",
            quant: "Q6_K",
            aa_score: Some(43),
            size_gb: 28.0,
            speed: "Usable (10–30 tok/s)",
            why: "Same top MoE at higher quant; better fidelity for 30–40 GB EIM",
            install: "llama-cli -hf unsloth/Qwen3.6-35B-A3B-GGUF:UD-Q6_K",
        },
        6 => TierModel {
            name: "Nemotron 3 Super",
            quant: "Q4_K_M",
            aa_score: Some(36),
            size_gb: 47.0,
            speed: "Slow (2–10 tok/s)",
            why: "Highest AA score in the 45–65 GB EIM tier",
            install: "ollama run nemotron3:super",
        },
        7 => TierModel {
            name: "Qwen3.5-122B-A10B",
            quant: "Q4_K_M",
            aa_score: Some(42),
            size_gb: 73.0,
            speed: "Slow (2–10 tok/s)",
            why: "MoE 122B with AA score 42; best available for 70–95 GB EIM",
            install: "ollama run qwen3.5:122b-a10b-q4_k_m",
        },
        8 => TierModel {
            name: "Qwen3.5-122B-A10B",
            quant: "Q6_K",
            aa_score: Some(42),
            size_gb: 97.0,
            speed: "Slow (2–10 tok/s)",
            why: "Higher-quant 122B for better fidelity on 100–150 GB EIM machines",
            install: "ollama run qwen3.5:122b-a10b-q6_k",
        },
        _ => TierModel {
            name: "Qwen3.5-397B-A17B",
            quant: "Q4_K_M",
            aa_score: None,
            size_gb: 238.0,
            speed: "Slow (2–10 tok/s)",
            why: "Frontier-class MoE; at hardware ceiling — no stretch above T9",
            install: "ollama run qwen3.5:397b-a17b-q4_k_m",
        },
    }
}

/// Returns three LLM recommendations: COMFORTABLE (tier−1), BALANCED (tier), STRETCH (tier+1).
pub fn recommend_models(profile: &HardwareProfile) -> serde_json::Value {
    let comfortable = best_for_tier(profile.tier.saturating_sub(1));
    let balanced = best_for_tier(profile.tier);
    let stretch = best_for_tier(std::cmp::min(profile.tier + 1, 9));

    let mut notes: Vec<String> = Vec::new();
    if profile.free_disk_gb < balanced.size_gb * 2.0 {
        notes.push(format!(
            "Low disk: {:.0} GB free, but balanced model ({}) needs ~{:.0} GB. Free space first.",
            profile.free_disk_gb,
            balanced.name,
            balanced.size_gb * 2.0
        ));
    }
    if profile.is_apple_silicon {
        notes.push(
            "Apple Silicon: GPU draws from unified RAM. EIM = ~75% of total RAM by default."
                .to_string(),
        );
        notes.push("Raise GPU cap: sudo sysctl iogpu.wired_limit_mb=<MB>".to_string());
    }
    if profile.eim_gb >= 16.0 {
        notes.push("At 256K context, budget 20–40 GB extra for KV cache.".to_string());
    }

    serde_json::json!({
        "hardware_detected": {
            "os": profile.os,
            "cpu": profile.cpu_model,
            "gpu": profile.gpu_model,
            "ram_gb": format!("{:.1}", profile.ram_gb),
            "free_disk_gb": format!("{:.1}", profile.free_disk_gb),
            "eim_gb": format!("{:.1}", profile.eim_gb),
            "tier": format!("T{}", profile.tier)
        },
        "recommendations": [
            {
                "rank": 1,
                "label": "COMFORTABLE",
                "model": comfortable.name,
                "quantization": comfortable.quant,
                "aa_score": comfortable.aa_score,
                "size_gb": comfortable.size_gb,
                "speed": comfortable.speed,
                "why": comfortable.why,
                "install": comfortable.install
            },
            {
                "rank": 2,
                "label": "BALANCED",
                "model": balanced.name,
                "quantization": balanced.quant,
                "aa_score": balanced.aa_score,
                "size_gb": balanced.size_gb,
                "speed": balanced.speed,
                "why": balanced.why,
                "install": balanced.install
            },
            {
                "rank": 3,
                "label": "STRETCH",
                "model": stretch.name,
                "quantization": stretch.quant,
                "aa_score": stretch.aa_score,
                "size_gb": stretch.size_gb,
                "speed": stretch.speed,
                "why": stretch.why,
                "install": stretch.install
            }
        ],
        "notes": notes
    })
}

/// Returns hardware specs as JSON — used by the get_hardware_specs tool.
pub fn hardware_specs_json(profile: &HardwareProfile) -> serde_json::Value {
    serde_json::json!({
        "os": profile.os,
        "cpu": profile.cpu_model,
        "gpu": profile.gpu_model,
        "vram_gb": format!("{:.1}", profile.vram_gb),
        "ram_gb": format!("{:.1}", profile.ram_gb),
        "free_disk_gb": format!("{:.1}", profile.free_disk_gb),
        "disk_path": profile.disk_path,
        "is_apple_silicon": profile.is_apple_silicon,
        "effective_inference_memory_gb": format!("{:.1}", profile.eim_gb),
        "llm_tier": format!("T{}", profile.tier)
    })
}
