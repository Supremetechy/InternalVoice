use std::fs;
use std::io::{self, Write};
use crate::config::{AppConfig, config_path};
use crate::error::Result;

pub fn run_interactive_setup() -> Result<()> {
    println!("\n  ── InternalVoice Setup Wizard ──\n");

    let path = config_path().unwrap_or_else(|_| std::path::PathBuf::from("config.toml"));
    
    if !path.exists() {
        println!("  [1/3] Creating config.toml...");
        let example = std::path::PathBuf::from("config/InternalVoice.example.toml");
        if example.exists() {
            fs::copy(&example, &path)?;
            println!("  [✓] Created config.toml from example template.");
        } else {
            let config = AppConfig::load().unwrap_or_else(|_| {

                // Return a default config if load fails and file doesn't exist
                // This is a bit complex since AppConfig doesn't have a full Default
                // I'll just write a minimal string for now
                AppConfig::load_from_str("[service]\nsample_interval_secs=60\ngemini_timeout_secs=30\ngemini_model=\"gemini-3.5-flash\"\ngemini_live_model=\"gemini-3.1-flash-live-preview\"\ngemini_api_key=\"\"\ndebounce_millis=500\nmax_prompt_tokens_hint=1024\ncache_ttl_secs=3600\nmax_log_files=5\nlog_dir=\"logs\"\nmax_log_file_size_mb=10\n[secrets]\nenv_var=\"GEMINI_API_KEY\"\nkeyring_service=\"internalvoice\"\nkeyring_account=\"default\"\n[limits]\nrequests_per_minute=15\nmax_cpu_percent=80.0\nmax_memory_mb=512\nbreaker_failure_threshold=3\nbreaker_reset_secs=60\n[narration]\nmin_announcement_interval_secs=300\nvoice_enabled=true\nmax_processes=5\nstyle=\"concise and helpful\"\n[policy]\nnotify_on_battery_below=20.0\nnotify_on_disk_below=10.0\nnotify_on_cpu_above=90.0\nnotify_on_memory_above=90.0\n").unwrap()
            });
            config.save()?;
            println!("  [✓] Created minimal config.toml.");
        }
    } else {
        println!("  [1/3] config.toml already exists. Updating existing configuration.");
    }

    let mut config = AppConfig::load()?;

    print!("  [2/3] Enter your Gemini API Key (press Enter to skip): ");
    io::stdout().flush()?;
    let mut api_key = String::new();
    io::stdin().read_line(&mut api_key)?;
    let api_key = api_key.trim();

    if !api_key.is_empty() {
        config.service.gemini_api_key = api_key.to_string();
    }

    print!("  [3/3] Enable Voice Narration? (y/n, default: y): ");
    io::stdout().flush()?;
    let mut voice = String::new();
    io::stdin().read_line(&mut voice)?;
    let voice = voice.trim().to_lowercase();
    
    if voice == "n" {
        config.narration.voice_enabled = false;
    } else if voice == "y" || voice.is_empty() {
        config.narration.voice_enabled = true;
    }

    config.save()?;
    println!("\n  [✓] Configuration saved to {}", path.display());
    println!("  [✓] Setup Wizard complete.\n");

    Ok(())
}
