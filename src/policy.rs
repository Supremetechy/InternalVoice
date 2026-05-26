use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{AppConfig, NotificationFrequency},
    state::model::{AlertSeverity, SystemState},
};

#[derive(Debug, Clone, Serialize)]
pub struct NarrativePrompt {
    pub cache_key: String,
    pub prompt: String,
    pub should_speak: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpokenMessage {
    pub timestamp: DateTime<Utc>,
    pub utterance: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterruptionDecision {
    pub speak: bool,
    pub utterance: Option<String>,
    pub priority: Option<String>,
    #[serde(default)]
    pub setup_info: Option<SetupInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupInfo {
    pub frequency: NotificationFrequency,
    pub completed: bool,
}

pub struct PolicyEngine {
    last_announcement: Option<Instant>,
    last_periodic_announcement: Option<DateTime<Utc>>,
    state_history: VecDeque<SystemState>,
    message_history: VecDeque<SpokenMessage>,
    max_history: usize,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self {
            last_announcement: None,
            last_periodic_announcement: None,
            state_history: VecDeque::with_capacity(10),
            message_history: VecDeque::with_capacity(5),
            max_history: 10,
        }
    }

    pub fn record_spoken(&mut self, utterance: String) {
        if self.message_history.len() >= 5 {
            self.message_history.pop_front();
        }
        self.message_history.push_back(SpokenMessage {
            timestamp: Utc::now(),
            utterance,
        });
    }

    pub fn system_instructions(&self, config: &AppConfig) -> String {
        let style_instruction = format!("Your style is {}.", config.narration.style);
        
        let skill_file = std::path::Path::new(".sixth/skills/it_specialist.md");
        let skill_content = if skill_file.exists() {
            std::fs::read_to_string(skill_file).unwrap_or_default()
        } else {
            String::new()
        };

        let mut instructions = vec![
            "You are InternalVoice, an AI System Monitor & Assistant.",
            &style_instruction,
        ];

        if !skill_content.is_empty() {
            instructions.push(&skill_content);
        } else {
            instructions.push("You are a system monitor narrator.");
        }

        instructions.extend(vec![
            "Decide if the user needs to be interrupted.",
            "Summarize only actionable system conditions.",
            "Do not suggest shell commands.",
            "Produce a short, spoken-friendly summary (1–2 sentences).",
            "Optionally suggest a single action (e.g., close app X, plug in power, free disk).",
            "Avoid repeating the same warning unless the situation worsens significantly.",
        ]);

        if !config.alerts.setup_completed {
            instructions.push("CRITICAL: The user has not set up their alert preferences yet.");
            instructions.push("You MUST prompt the user to choose a notification frequency: 'daily', 'hourly', or 'minute'.");
            instructions.push("If they respond with a preference, include it in your JSON response under 'setup_info'.");
        }

        instructions.push("When providing updates, also include suggestions on ways to optimize system resources based on the current state.");

        instructions.join("\n")
    }

    pub fn build_setup_prompt(&self, config: &AppConfig) -> NarrativePrompt {
        let prompt = serde_json::json!({
            "role": "system monitor narrator",
            "style": config.narration.style,
            "goal": "Prompt the user to setup their system preferences for alert notifications.",
            "instructions": [
                "Ask the user if they would like daily, hourly, or minute-by-minute updates of their system health and resources.",
                "Explain that you will provide updates on health, resources, and running processes, along with optimization suggestions.",
                "Output your decision in RAW JSON format. No markdown blocks."
            ],
            "output_format": {
                "speak": "boolean",
                "utterance": "string",
                "priority": "high",
                "setup_info": {
                    "frequency": "daily|hourly|minute|none",
                    "completed": "boolean"
                }
            }
        });

        NarrativePrompt {
            cache_key: "setup_prompt".into(),
            prompt: prompt.to_string(),
            should_speak: config.narration.voice_enabled,
        }
    }

    pub fn build_prompt(
        &mut self,
        state: &SystemState,
        config: &AppConfig,
    ) -> Option<NarrativePrompt> {
        // Record state in history
        if self.state_history.len() >= self.max_history {
            self.state_history.pop_front();
        }
        self.state_history.push_back(state.clone());

        let is_periodic = self.check_periodic(config);
        
        if state.alerts.is_empty() && !is_resource_pressure(state, config) && !is_periodic && config.alerts.setup_completed {
            return None;
        }

        let can_announce = self
            .last_announcement
            .map(|last| last.elapsed() >= config.announcement_interval())
            .unwrap_or(true);

        if !can_announce && !is_periodic {
            return None;
        }

        self.last_announcement = Some(Instant::now());
        if is_periodic {
            self.last_periodic_announcement = Some(Utc::now());
        }
        
        let prompt = serde_json::json!({
            "role": "system monitor narrator",
            "style": config.narration.style,
            "goal": "Decide if the user needs to be interrupted or provide a periodic update.",
            "instructions": [
                "Summarize only actionable system conditions.",
                "Provide suggestions to user on ways to optimize their system resources.",
                "Do not suggest shell commands.",
                "Produce a short, spoken-friendly summary (1–2 sentences).",
                "Optionally suggest a single action (e.g., close app X, plug in power, free disk).",
                "Avoid repeating the same warning unless the situation worsens significantly.",
                "Output your decision in RAW JSON format. No markdown blocks."
            ],
            "is_periodic_update": is_periodic,
            "policies": {
                "telemetry": config.policy.system_telemetry,
                "thresholds": {
                    "cpu_above": config.policy.notify_on_cpu_above,
                    "memory_above": config.policy.notify_on_memory_above,
                    "battery_below": config.policy.notify_on_battery_below,
                    "disk_below": config.policy.notify_on_disk_below
                }
            },
            "context": {
                "latest_state": state,
                "recent_history": self.state_history.iter().take(self.state_history.len() - 1).collect::<Vec<_>>(),
                "last_messages_spoken": self.message_history.iter().collect::<Vec<_>>()
            },
            "output_format": {
                "speak": "boolean",
                "utterance": "string | null",
                "priority": "low|medium|high | null",
                "setup_info": {
                    "frequency": "daily|hourly|minute|none",
                    "completed": "boolean"
                }
            }
        });

        Some(NarrativePrompt {
            cache_key: format!(
                "{}:{}:{}:{}",
                state.collected_at.timestamp(),
                state.alerts.len(),
                self.message_history.len(),
                is_periodic
            ),
            prompt: prompt.to_string(),
            should_speak: config.narration.voice_enabled,
        })
    }

    fn check_periodic(&self, config: &AppConfig) -> bool {
        if config.alerts.frequency == NotificationFrequency::None {
            return false;
        }

        let last = match self.last_periodic_announcement {
            Some(l) => l,
            None => return true, // Trigger first one immediately if frequency is set
        };

        let now = Utc::now();
        let diff = now - last;

        match config.alerts.frequency {
            NotificationFrequency::Minute => diff.num_minutes() >= 1,
            NotificationFrequency::Hourly => diff.num_hours() >= 1,
            NotificationFrequency::Daily => diff.num_days() >= 1,
            NotificationFrequency::None => false,
        }
    }
}

fn is_resource_pressure(state: &SystemState, config: &AppConfig) -> bool {
    state.cpu.total_usage_percent >= config.policy.notify_on_cpu_above
        || state.memory.used_percent >= config.policy.notify_on_memory_above
        || state
            .battery
            .as_ref()
            .map(|b| b.percent <= config.policy.notify_on_battery_below)
            .unwrap_or(false)
        || state
            .alerts
            .iter()
            .any(|alert| matches!(alert.severity, AlertSeverity::Critical))
}

#[allow(dead_code)]
fn _duration_to_millis(value: Duration) -> u128 {
    value.as_millis()
}
