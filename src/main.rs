mod audio;
mod cache;
mod config;
mod context;
mod error;
mod gemini;
mod policy;
mod platform;
mod publisher;
mod security;
mod sensors;
mod state;
mod tts;

use std::{fs, sync::Arc};
use futures_util::{SinkExt, StreamExt};
use tokio::{signal, time::sleep};
use tracing::{error, info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{
    config::AppConfig,
    context::ServiceContext,
    error::Result,
    gemini::{GeminiLiveClient, ServerMessage},
    security::{enforce_least_privilege, validate_action, validate_config, AllowedAction},
};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let config = AppConfig::load()?;
    validate_config(&config)?;
    init_tracing(&config)?;
    enforce_least_privilege();

    let context = Arc::new(ServiceContext::initialize(config)?);
    info!("InternalVoice starting");

    tokio::select! {
        result = run_service(context.clone()) => {
            if let Err(err) = result {
                error!(error = %err, "service loop failed");
                return Err(err);
            }
        }
        _ = shutdown_signal() => {
            info!("shutdown signal received");
        }
    }

    info!("InternalVoice stopped");
    Ok(())
}

async fn run_service(context: Arc<ServiceContext>) -> Result<()> {
    let system_instruction = {
        let policy = context.policy.lock().await;
        let config = context.config.lock().await;
        Some(policy.system_instructions(&config))
    };

    match context.gemini_live.connect(system_instruction).await {
        Ok(mut ws_stream) => {
            info!("Established Gemini Live duplex transport");
            
            let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<String>(100);
            let _recording_stream = context.audio_engine.start_recording(audio_tx)?;

            // Initial setup check
            {
                let config = context.config.lock().await;
                if !config.alerts.setup_completed {
                    let setup_prompt = {
                        let policy = context.policy.lock().await;
                        policy.build_setup_prompt(&config)
                    };
                    let setup_msg = serde_json::json!({
                        "client_content": {
                            "turns": [{
                                "role": "user",
                                "parts": [{"text": setup_prompt.prompt}]
                            }],
                            "turn_complete": true
                        }
                    });
                    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(setup_msg.to_string().into())).await.map_err(|e| {
                        crate::error::InternalVoiceError::Gemini(format!("Failed to send setup prompt: {}", e))
                    })?;
                }
            }

            loop {
                tokio::select! {
                    Some(audio_chunk) = audio_rx.recv() => {
                        if let Err(e) = GeminiLiveClient::send_audio_chunk(&mut ws_stream, &audio_chunk).await {
                            error!(error = %e, "Failed to send audio chunk");
                            break;
                        }
                    }
                    Some(msg) = ws_stream.next() => {
                        match msg {
                            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                                match serde_json::from_str::<ServerMessage>(&text) {
                                    Ok(ServerMessage::RealtimeInput { media_chunks }) => {
                                        for chunk in media_chunks {
                                            if let Err(e) = context.audio_engine.play_audio(&chunk.data) {
                                                error!(error = %e, "Failed to play audio chunk");
                                            }
                                        }
                                    }
                                    Ok(ServerMessage::ServerContent { model_turn }) => {
                                        for part in model_turn.parts {
                                            if let Some(text) = part.text {
                                                info!(text = %text, "InternalVoice said");
                                                
                                                // Check for setup info in JSON if the text contains JSON
                                                if let Ok(decision) = serde_json::from_str::<crate::policy::InterruptionDecision>(&text) {
                                                    if let Some(setup) = decision.setup_info {
                                                        let mut config = context.config.lock().await;
                                                        config.alerts.frequency = setup.frequency;
                                                        config.alerts.setup_completed = setup.completed;
                                                        config.save()?;
                                                        info!(frequency = ?config.alerts.frequency, "Alert preferences updated");
                                                    }
                                                    if decision.speak {
                                                        if let Some(utterance) = decision.utterance {
                                                            if context.config.lock().await.narration.voice_enabled {
                                                                context.tts.speak(&utterance).await;
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    // Fallback for regular text
                                                    if context.config.lock().await.narration.voice_enabled {
                                                        context.tts.speak(&text).await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Ok(msg) => info!("Received server message: {:?}", msg),
                                    Err(e) => error!(error = %e, "Failed to parse server message"),
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "WebSocket error");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ = sleep(Duration::from_millis(100)) => {
                        // Periodic state updates could be sent here as well
                    }
                    _ = shutdown_signal() => {
                        info!("shutdown signal received");
                        break;
                    }
                }
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to connect to Gemini Live API, falling back to polling");
            

            run_polling_service(context).await?;
        }

    }

    Ok(())
}

async fn run_polling_service(context: Arc<ServiceContext>) -> Result<()> {
    loop {
        let (state, config) = {
            let mut service = context.state_service.lock().await;
            let config = context.config.lock().await;
            (service.sample(&config), config.clone())
        };
        context.publisher.publish(&state)?;

        let prompt = {
            let mut policy = context.policy.lock().await;
            policy.build_prompt(&state, &config)
        };

        if let Some(prompt) = prompt {
            if !validate_action(&AllowedAction::Announce) {
                warn!("announcement action was rejected by allow-list");
            } else if let Some(cached) = context.cache.get(&prompt.prompt)? {
                info!(summary = %cached, "cache hit");
                if let Ok(decision) = serde_json::from_str::<crate::policy::InterruptionDecision>(&cached) {
                    process_decision(context.clone(), decision).await?;
                }
            } else {
                match context
                    .gemini
                    .generate_summary(&prompt.prompt, config.service.max_prompt_tokens_hint)
                    .await
                {
                    Ok(raw_json) => {
                        match serde_json::from_str::<crate::policy::InterruptionDecision>(&raw_json) {
                            Ok(decision) => {
                                process_decision(context.clone(), decision).await?;
                                context.cache.put(&prompt.prompt, &raw_json)?;
                            }
                            Err(e) => {
                                // Gemini may return a wrapper object (e.g. {"output_format": { ... }})
                                // instead of the raw InterruptionDecision.
                                #[derive(serde::Deserialize)]
                                struct GeminiWrapper {
                                    #[serde(default)]
                                    output_format: Option<crate::policy::InterruptionDecision>,
                                }

                                // Strip common markdown fences like ```json ... ``` that models may return.
                                let cleaned = raw_json
                                    .trim()
                                    .trim_start_matches("```json")
                                    .trim_start_matches("```")
                                    .trim_end_matches("```")
                                    .trim();

                                if let Ok(wrapper) = serde_json::from_str::<GeminiWrapper>(cleaned) {

                                    if let Some(decision) = wrapper.output_format {
                                        process_decision(context.clone(), decision).await?;
                                        context.cache.put(&prompt.prompt, &raw_json)?;
                                        continue;
                                    }
                                }

                                warn!(
                                    error = %e,
                                    raw_response = %raw_json,
                                    "Failed to parse interruption decision"
                                );
                            }

                        }

                    }
                    Err(err) => warn!(error = %err, "Gemini request failed"),
                }
            }
        }

        let interval = context.config.lock().await.sample_interval();
        sleep(interval).await;
    }
}

async fn process_decision(context: Arc<ServiceContext>, decision: crate::policy::InterruptionDecision) -> Result<()> {
    if let Some(setup) = decision.setup_info {
        let mut config = context.config.lock().await;
        config.alerts.frequency = setup.frequency;
        config.alerts.setup_completed = setup.completed;
        config.save()?;
        info!(frequency = ?config.alerts.frequency, "Alert preferences updated");
    }

    if decision.speak {
        if let Some(utterance) = decision.utterance {
            info!(text = %utterance, priority = ?decision.priority, "InternalVoice said");
            if context.config.lock().await.narration.voice_enabled {
                context.tts.speak(&utterance).await;
            }
            let mut policy = context.policy.lock().await;
            policy.record_spoken(utterance);
        }
    }
    Ok(())
}

use std::time::Duration;

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal as unix_signal, SignalKind};

        let mut sigterm = unix_signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = signal::ctrl_c().await;
    }
}

fn init_tracing(config: &AppConfig) -> Result<()> {
    let log_dir = config.log_dir();
    fs::create_dir_all(&log_dir)?;

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("internalvoice")
        .filename_suffix("log")
        .max_log_files(config.max_log_files())
        .build(log_dir)?;

    info!(
        log_dir = %config.log_dir().display(),
        max_files = config.max_log_files(),
        max_size_mb = config.max_log_file_size() / 1024 / 1024,
        "log system initialized"
    );

    let stdout_layer = fmt::layer().with_target(false);
    let file_layer = fmt::layer().with_writer(appender).with_ansi(false);

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive("internalvoice=info".parse().unwrap()))
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(())
}
