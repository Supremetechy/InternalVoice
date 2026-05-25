use thiserror::Error;

#[derive(Debug, Error)]
pub enum InternalVoiceError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("secret lookup failed: {0}")]
    Secret(String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("gemini api error: {0}")]
    Gemini(String),
    #[error("audio error: {0}")]
    Audio(String),
    #[error("tracing error: {0}")]
    Tracing(String),
}

pub type Result<T> = std::result::Result<T, InternalVoiceError>;

impl From<tracing_appender::rolling::InitError> for InternalVoiceError {
    fn from(err: tracing_appender::rolling::InitError) -> Self {
        InternalVoiceError::Tracing(err.to_string())
    }
}

