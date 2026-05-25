use std::{fs, sync::Arc, time::Duration};

use tokio::sync::Mutex;

use crate::{
    audio::AudioEngine,
    cache::PromptCache,
    config::{data_dir, AppConfig},
    error::Result,
    gemini::{GeminiClient, GeminiLiveClient},
    policy::PolicyEngine,
    publisher::StatePublisher,
    security::SecretProvider,
    state::sampler::SystemStateService,
    tts::TtsEngine,
};

pub struct ServiceContext {
    pub config: Arc<Mutex<AppConfig>>,
    pub gemini: GeminiClient,
    pub gemini_live: GeminiLiveClient,
    pub audio_engine: Arc<AudioEngine>,
    pub cache: Arc<PromptCache>,
    pub state_service: Arc<Mutex<SystemStateService>>,
    pub policy: Arc<Mutex<PolicyEngine>>,
    pub publisher: Arc<StatePublisher>,
    pub tts: Arc<TtsEngine>,
}

impl ServiceContext {
    pub fn initialize(config: AppConfig) -> Result<Self> {
        let data_dir = data_dir()?;
        fs::create_dir_all(&data_dir)?;

        let secret_provider = SecretProvider::new(&config);
        let api_key = secret_provider.gemini_api_key()?;
        let gemini = GeminiClient::new(api_key.clone(), &config)?;
        let gemini_live = GeminiLiveClient::new(api_key, &config);
        let audio_engine = Arc::new(AudioEngine::new()?);

        let cache = PromptCache::open(
            &data_dir.join("prompt-cache.sqlite3"),
            Duration::from_secs(config.service.cache_ttl_secs),
        )?;
        let publisher = StatePublisher::new(data_dir.join("current-state.json"));

        Ok(Self {
            config: Arc::new(Mutex::new(config)),
            gemini,
            gemini_live,
            audio_engine,
            cache: Arc::new(cache),
            state_service: Arc::new(Mutex::new(SystemStateService::new())),
            policy: Arc::new(Mutex::new(PolicyEngine::new())),
            publisher: Arc::new(publisher),
            tts: Arc::new(TtsEngine::new()),
        })
    }
}
