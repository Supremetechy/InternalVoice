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
mod setup;
mod tools;
mod tts;

use std::{fs, sync::Arc};
use futures_util::{SinkExt, StreamExt};
use tokio::{signal, time::sleep};
use tracing::{debug, error, info, warn};
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

    // Check for --setup flag
    if std::env::args().any(|arg| arg == "--setup") {
        setup::run_interactive_setup()?;
        return Ok(());
    }

    let config = AppConfig::load()?;
    validate_config(&config)?;
    init_tracing(&config)?;
    enforce_least_privilege();

    print_banner();

    let context = Arc::new(ServiceContext::initialize(config)?);
    info!("InternalVoice starting");

    {
        let cfg = context.config.lock().await;
        if !cfg.alerts.setup_completed {
            print_first_run_setup(&context.audio_engine);
        } else {
            println!("  Microphone : {}", context.audio_engine.input_device_name().unwrap_or_else(|| "default".into()));
            println!("  Speaker    : {}", context.audio_engine.output_device_name().unwrap_or_else(|| "default".into()));
            println!();
        }
    }

    println!("  Connecting to Gemini Live...");

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

    println!("\n  InternalVoice stopped.");
    info!("InternalVoice stopped");
    Ok(())
}

async fn run_service(context: Arc<ServiceContext>) -> Result<()> {
    let system_instruction = {
        let policy = context.policy.lock().await;
        let config = context.config.lock().await;
        Some(policy.system_instructions(&config))
    };

    let output_rate = context.audio_engine.output_sample_rate();
    match context.gemini_live.connect(system_instruction, output_rate).await {
        Ok(mut ws_stream) => {
            info!("Established Gemini Live duplex transport");
            println!("  Connected.\n");

            let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<String>(100);
            let _recording_stream = context.audio_engine.start_recording(audio_tx)?;

            // On first run send the setup prompt so Gemini asks about notification frequency.
            {
                let config = context.config.lock().await;
                if !config.alerts.setup_completed {
                    println!("  Speak your notification frequency preference:");
                    println!("    \"daily\", \"hourly\", or \"every minute\"\n");
                    let setup_prompt = {
                        let policy = context.policy.lock().await;
                        policy.build_setup_prompt(&config)
                    };
                    let setup_msg = serde_json::json!({
                        "clientContent": {
                            "turns": [{"role": "user", "parts": [{"text": setup_prompt.prompt}]}],
                            "turnComplete": true
                        }
                    });
                    ws_stream.send(tokio_tungstenite::tungstenite::Message::Text(
                        setup_msg.to_string().into()
                    )).await.map_err(|e| {
                        crate::error::InternalVoiceError::Gemini(
                            format!("Failed to send setup prompt: {}", e)
                        )
                    })?;
                } else {
                    println!("  Listening for voice input. Press Ctrl+C to stop.\n");
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
                    msg = ws_stream.next() => {
                        match msg {
                            Some(Ok(m)) if m.is_text() || m.is_binary() => {
                                // Extract payload from either Text or Binary WebSocket frame.
                                let raw = match &m {
                                    tokio_tungstenite::tungstenite::Message::Text(s) => s.to_string(),
                                    tokio_tungstenite::tungstenite::Message::Binary(b) => {
                                        String::from_utf8_lossy(b).into_owned()
                                    }
                                    _ => continue,
                                };
                                let text = raw.trim();
                                if text.is_empty() || !text.starts_with('{') {
                                    continue;
                                }

                                match serde_json::from_str::<ServerMessage>(text) {
                                    Ok(ServerMessage::SetupComplete {}) => {
                                        info!("Gemini Live session ready");
                                        println!("  [InternalVoice] Session ready.");
                                    }
                                    Ok(ServerMessage::ServerContent { model_turn, turn_complete, .. }) => {
                                        if let Some(content) = model_turn {
                                            for part in content.parts {
                                                // Gemini Live is the voice — play its audio directly.
                                                // TTS is never used while the WebSocket is alive.
                                                if let Some(audio) = part.inline_data {
                                                    debug!("Received audio response from Gemini ({} bytes)", audio.data.len());
                                                    let rate = parse_audio_rate(&audio.mime_type);
                                                    if let Err(e) = context.audio_engine.play_audio(&audio.data, rate) {
                                                        error!(error = %e, "Failed to play audio response");
                                                    }
                                                }
                                                // Show any text content on the console only (no TTS).
                                                if let Some(response_text) = part.text {
                                                    if response_text.trim().is_empty() {
                                                        continue;
                                                    }
                                                    if let Ok(decision) = serde_json::from_str::<crate::policy::InterruptionDecision>(&response_text) {
                                                        if let Some(setup) = decision.setup_info {
                                                            let mut config = context.config.lock().await;
                                                            config.alerts.frequency = setup.frequency;
                                                            config.alerts.setup_completed = setup.completed;
                                                            config.save()?;
                                                            println!("  [Setup] Alert frequency set to {:?}.", config.alerts.frequency);
                                                            info!(frequency = ?config.alerts.frequency, "Alert preferences updated");
                                                        }
                                                        if let Some(utterance) = decision.utterance {
                                                            println!("  [InternalVoice] {}", utterance);
                                                            info!(text = %utterance, "InternalVoice said");
                                                        }
                                                    } else {
                                                        println!("  [InternalVoice] {}", response_text);
                                                        info!(text = %response_text, "InternalVoice said");
                                                    }
                                                }
                                            }
                                        }
                                        if turn_complete {
                                            debug!("Turn complete");
                                            context.audio_engine.notify_turn_complete();
                                        }
                                    }
                                    Ok(ServerMessage::RealtimeInput { audio }) => {
                                        if let Some(chunk) = audio {
                                            let rate = parse_audio_rate(&chunk.mime_type);
                                            if let Err(e) = context.audio_engine.play_audio(&chunk.data, rate) {
                                                error!(error = %e, "Failed to play audio chunk");
                                            }
                                        }
                                    }

                                    Ok(ServerMessage::ToolCall { function_calls }) => {
                                        info!(count = function_calls.len(), "Received tool calls from Gemini");
                                        let mut responses = Vec::new();
                                        for call in function_calls {
                                            match crate::tools::call_tool(&call.name, call.args) {
                                                Ok(response) => {
responses.push(crate::gemini::FunctionResponse {
                                                        name: call.name,
                                                        id: call.id,
                                                        response,
                                                    });
                                                }
                                                Err(e) => {
                                                    error!(error = %e, tool = %call.name, "Tool execution failed");
responses.push(crate::gemini::FunctionResponse {
                                                        name: call.name,
                                                        id: call.id,
                                                        response: serde_json::json!({ "error": e.to_string() }),
                                                    });
                                                }
                                            }
                                        }

let response_msg = crate::gemini::ToolResponseMessage {
                                            tool_response: crate::gemini::ToolResponseContent {
                                                function_responses: responses,
                                            },
                                        };


                                        if let Ok(msg_text) = serde_json::to_string(&response_msg) {
                                            debug!(payload = %msg_text, "Sending Gemini Live tool response payload");
                                            if let Err(e) = ws_stream
                                                .send(tokio_tungstenite::tungstenite::Message::Text(msg_text.into()))
                                                .await

                                            {
                                                error!(error = %e, "Failed to send tool response to Gemini");
                                            }
                                        }

                                    }
                                    Err(e) => debug!(error = %e, "Ignoring unparseable server message"),
                                }
                            }
                            Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => {
                                let reason = frame
                                    .as_ref()
                                    .map(|f| f.reason.to_string())
                                    .unwrap_or_else(|| "no reason given".into());
                                warn!(reason = %reason, "WebSocket closed by server");
                                eprintln!("\n  [ERROR] Gemini Live disconnected: {}", reason);
                                if reason.to_lowercase().contains("api key") || reason.to_lowercase().contains("leaked") {
                                    eprintln!("  [ACTION] Regenerate your API key at https://aistudio.google.com/apikey");
                                    eprintln!("           Then update GEMINI_API_KEY in your .env file and restart.");
                                }
                                break;
                            }
                            Some(Ok(_)) => {} // Ping/Pong — ignore silently
                            Some(Err(e)) => {
                                error!(error = %e, "WebSocket error");
                                eprintln!("\n  [ERROR] WebSocket error: {}", e);
                                break;
                            }
                            None => {
                                warn!("WebSocket stream ended");
                                eprintln!("\n  [ERROR] Connection dropped unexpectedly.");
                                break;
                            }
                        }
                    }
                    _ = sleep(Duration::from_millis(100)) => {}
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
                    Err(err) => {
                        warn!(error = %err, "Gemini request failed");
                        let msg = err.to_string();
                        if msg.contains("leaked") || msg.contains("API key") || msg.contains("403") {
                            eprintln!("\n  [ERROR] Gemini API rejected the request: {}", msg);
                            eprintln!("  [ACTION] Regenerate your API key at https://aistudio.google.com/apikey");
                            eprintln!("           Then update GEMINI_API_KEY in your .env file and restart.");
                            break Ok(());
                        }
                    }
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

/// Extracts the sample rate from a MIME type string such as `audio/pcm;rate=24000`.
/// Falls back to 24000 (Gemini Live default) when the field is absent or unparseable.
fn parse_audio_rate(mime_type: &str) -> u32 {
    mime_type
        .split(';')
        .find_map(|segment| {
            let s = segment.trim();
            s.strip_prefix("rate=").and_then(|v| v.parse().ok())
        })
        .unwrap_or(24000)
}

fn print_banner() {
    println!();
    println!("  ┌─────────────────────────────────────────────────┐");
    println!("  │           I N T E R N A L  V O I C E           │");
    println!("  │         AI System Monitor & Assistant            │");
    println!("  └─────────────────────────────────────────────────┘");
    println!();
}

fn print_first_run_setup(audio: &crate::audio::AudioEngine) {
    println!("  ── First-Run Setup ───────────────────────────────");
    println!();
    println!("  Speech-to-Text (STT)");
    println!("    Engine  : Gemini Live (real-time voice recognition)");
    if let Some(name) = audio.input_device_name() {
        println!("    Mic     : {}", name);
    }
    println!();
    println!("  Text-to-Speech (TTS)");
    #[cfg(target_os = "macos")]
    println!("    Engine  : macOS 'say' command");
    #[cfg(target_os = "windows")]
    println!("    Engine  : Windows Speech Synthesis");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    println!("    Engine  : espeak / festival");
    if let Some(name) = audio.output_device_name() {
        println!("    Speaker : {}", name);
    }
    println!();
    println!("  Notification Frequency");
    println!("    InternalVoice will ask via voice — listen for the prompt.");
    println!("    Speak one of: \"daily\"  |  \"hourly\"  |  \"every minute\"");
    println!();
    println!("  ──────────────────────────────────────────────────");
    println!();
}

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

    // No stdout tracing layer — user-facing output uses println!.
    // Tracing goes to the rotating log file only (debug+ from our crate).
    let file_layer = fmt::layer().with_writer(appender).with_ansi(false);

    tracing_subscriber::registry()
        .with(
            EnvFilter::from_default_env()
                .add_directive("internalvoice=debug".parse().unwrap())
                .add_directive("warn".parse().unwrap()),
        )
        .with(file_layer)
        .init();

    info!(log_dir = %config.log_dir().display(), "log system initialized");

    Ok(())
}
