use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::protocol::Message,
    MaybeTlsStream, WebSocketStream,
};

use crate::{
    config::AppConfig,
    error::{InternalVoiceError, Result},
};

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

#[derive(Clone)]
pub struct GeminiClient {
    http: Client,
    api_key: Arc<String>,
    model: Arc<String>,
    limiter: Arc<Limiter>,
    breaker: Arc<Mutex<CircuitBreaker>>,
    timeout: Duration,
    breaker_failure_threshold: u32,
    breaker_reset: Duration,
    debounce_interval: Duration,
}

#[derive(Debug, Default)]
struct CircuitBreaker {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

#[derive(Debug, Serialize)]
struct StreamRequest<'a> {
    contents: [Content<'a>; 1],
    generation_config: GenerationConfig,
}

#[derive(Debug, Serialize)]
struct Content<'a> {
    role: &'a str,
    parts: [Part<'a>; 1],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<Blob<'a>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blob<'a> {
    pub mime_type: &'a str,
    pub data: &'a str,
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    temperature: f32,
    max_output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    #[serde(default)]
    content: Option<ResponseContent>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseContent {
    #[serde(default)]
    pub parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponsePart {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub inline_data: Option<ResponseBlob>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseBlob {
    #[allow(dead_code)]
    pub mime_type: String,
    pub data: String,
}

// Gemini Live API Structures
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSetup {
    setup: SetupConfigLive,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupConfigLive {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfigLive>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfigLive {
    #[serde(skip_serializing_if = "Option::is_none")]
    speech_config: Option<SpeechConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_modalities: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeechConfig {
    voice_config: VoiceConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceConfig {
    prebuilt_voice_config: PrebuiltVoiceConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrebuiltVoiceConfig {
    voice_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemInstruction {
    role: String,
    parts: Vec<PartStatic>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PartStatic {
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ClientContentMessage<'a> {
    client_content: ClientContent<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ClientContent<'a> {
    turns: Vec<Content<'a>>,
    turn_complete: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeInputMessage<'a> {
    realtime_input: RealtimeInput<'a>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeInput<'a> {
    media_chunks: Vec<Blob<'a>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerMessage {
    SetupComplete {},
    ServerContent {
        #[serde(default, rename = "modelTurn")]
        model_turn: Option<ResponseContent>,
        #[serde(default, rename = "turnComplete")]
        turn_complete: bool,
        #[serde(default, rename = "generationComplete")]
        #[allow(dead_code)]
        generation_complete: bool,
    },
    RealtimeInput {
        #[serde(rename = "mediaChunks")]
        media_chunks: Vec<ResponseBlob>,
    },
    #[allow(dead_code)]
    ToolCall {
        #[serde(rename = "functionCalls")]
        function_calls: Vec<serde_json::Value>,
    },
}

pub struct GeminiLiveClient {
    api_key: String,
    model: String,
    ws_url_template: String,
}

impl GeminiLiveClient {
    pub fn new(api_key: String, config: &AppConfig) -> Self {
        Self {
            api_key,
            model: config.service.gemini_live_model.clone(),
            ws_url_template: config.service.gemini_live_ws_url.clone(),
        }
    }

    pub async fn connect(
        &self,
        system_instruction: Option<String>,
        _output_sample_rate: u32,
    ) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
        let url = self.ws_url_template.replace("{key}", &self.api_key);
        let redacted_url = url.replace(&self.api_key, "<redacted>");

        let (mut ws_stream, _) = connect_async(url).await.map_err(|e| {
            InternalVoiceError::Gemini(format!(
                "WebSocket connection failed url={} error={}",
                redacted_url, e
            ))
        })?;

        let setup = LiveSetup {
            setup: SetupConfigLive {
                model: format!("models/{}", self.model),
                generation_config: Some(GenerationConfigLive {
                    speech_config: Some(SpeechConfig {
                        voice_config: VoiceConfig {
                            prebuilt_voice_config: PrebuiltVoiceConfig {
                                voice_name: "Puck".into(),
                            },
                        },
                    }),
                    response_modalities: Some(vec!["AUDIO".into()]),
                }),
                system_instruction: system_instruction.map(|text| SystemInstruction {
                    role: "system".into(),
                    parts: vec![PartStatic { text }],
                }),
            },
        };

        let msg = serde_json::to_string(&setup)?;
        ws_stream.send(Message::Text(msg.into())).await.map_err(|e| {
            InternalVoiceError::Gemini(format!("Failed to send setup message: {}", e))
        })?;

        Ok(ws_stream)
    }

    pub async fn send_audio_chunk(
        ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        base64_audio: &str
    ) -> Result<()> {
        let input = RealtimeInputMessage {
            realtime_input: RealtimeInput {
                media_chunks: vec![Blob {
                    mime_type: "audio/pcm;rate=16000",
                    data: base64_audio,
                }],
            },
        };

        let msg = serde_json::to_string(&input)?;
        ws_stream.send(Message::Text(msg.into())).await.map_err(|e| {
            InternalVoiceError::Gemini(format!("Failed to send audio chunk: {}", e))
        })?;

        Ok(())
    }
}

impl GeminiClient {
    pub fn new(api_key: String, config: &AppConfig) -> Result<Self> {
        let http = Client::builder()
            .use_rustls_tls()
            .pool_idle_timeout(Duration::from_secs(90))
            .timeout(config.gemini_timeout())
            .build()?;

        let quota = Quota::per_minute(
            std::num::NonZeroU32::new(config.limits.requests_per_minute).ok_or_else(|| {
                InternalVoiceError::Config("requests_per_minute must be > 0".into())
            })?,
        );
        Ok(Self {
            http,
            api_key: Arc::new(api_key),
            model: Arc::new(config.service.gemini_model.clone()),
            limiter: Arc::new(RateLimiter::direct(quota)),
            breaker: Arc::new(Mutex::new(CircuitBreaker::default())),
            timeout: config.gemini_timeout(),
            breaker_failure_threshold: config.limits.breaker_failure_threshold,
            breaker_reset: config.breaker_reset(),
            debounce_interval: config.debounce_interval(),
        })
    }

    pub async fn generate_summary(&self, prompt: &str, max_output_tokens: u32) -> Result<String> {
        self.await_turn().await?;
        self.ensure_breaker_closed()?;

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            self.model,
            self.api_key
        );

        let request = StreamRequest {
            contents: [Content {
                role: "user",
                parts: [Part { text: Some(prompt), inline_data: None }],
            }],
            generation_config: GenerationConfig {
                temperature: 0.3,
                max_output_tokens,
            },
        };

        let response = timeout(self.timeout, self.http.post(url).json(&request).send())
            .await
            .map_err(|_| InternalVoiceError::Gemini("request timed out".into()))??;
        if response.status().is_server_error() {
            self.record_failure();
            return Err(InternalVoiceError::Gemini(format!(
                "server returned {}",
                response.status()
            )));
        }

        if response.status() != StatusCode::OK {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            self.record_failure();
            return Err(InternalVoiceError::Gemini(format!(
                "request failed with {}: {}",
                status, body
            )));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut line_buffer = String::new();
        let started = Instant::now();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            let payload = String::from_utf8_lossy(&bytes);
            line_buffer.push_str(&payload);

            while let Some(newline_idx) = line_buffer.find('\n') {
                let line = line_buffer[..newline_idx].to_string();
                line_buffer = line_buffer[newline_idx + 1..].to_string();

                if let Some(data) = line.strip_prefix("data: ") {
                    if data.trim() == "[DONE]" {
                        self.record_success();
                        return Ok(buffer.trim().to_string());
                    }

                    match serde_json::from_str::<StreamChunk>(data) {
                        Ok(parsed) => {
                            for candidate in parsed.candidates {
                                if let Some(content) = candidate.content {
                                    for part in content.parts {
                                        if let Some(text) = part.text {
                                            buffer.push_str(&text);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            // If serialization fails, it might be due to a truncated chunk.
                            // However, with proper line buffering, this should be rare.
                            tracing::warn!(error = %e, data = %data, "Failed to parse SSE data chunk");
                        }
                    }
                }
            }

            if started.elapsed() > self.timeout {
                self.record_failure();
                return Err(InternalVoiceError::Gemini("stream timed out".into()));
            }
        }

        self.record_success();
        Ok(buffer.trim().to_string())
    }

    async fn await_turn(&self) -> Result<()> {
        self.limiter.until_ready().await;
        sleep(self.debounce_interval).await;
        Ok(())
    }

    fn ensure_breaker_closed(&self) -> Result<()> {
        let mut breaker = self.breaker.lock().expect("breaker mutex poisoned");
        if let Some(deadline) = breaker.open_until {
            if Instant::now() < deadline {
                return Err(InternalVoiceError::Gemini(
                    "circuit breaker is open after repeated failures".into(),
                ));
            }

            breaker.open_until = None;
            breaker.consecutive_failures = 0;
        }
        Ok(())
    }

    fn record_success(&self) {
        let mut breaker = self.breaker.lock().expect("breaker mutex poisoned");
        breaker.consecutive_failures = 0;
        breaker.open_until = None;
    }

    fn record_failure(&self) {
        let mut breaker = self.breaker.lock().expect("breaker mutex poisoned");
        breaker.consecutive_failures += 1;
        if breaker.consecutive_failures >= self.breaker_failure_threshold {
            breaker.open_until = Some(Instant::now() + self.breaker_reset);
        }
    }
}
