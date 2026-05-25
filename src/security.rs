use keyring::Entry;

use crate::{
    config::AppConfig,
    error::{InternalVoiceError, Result},
};

#[derive(Debug, Clone)]
pub enum AllowedAction {
    Announce,
    #[allow(dead_code)]
    QuerySystemState,
    #[allow(dead_code)]
    SummarizeTopProcesses,
}

pub struct SecretProvider<'a> {
    config: &'a AppConfig,
}

impl<'a> SecretProvider<'a> {
    pub fn new(config: &'a AppConfig) -> Self {
        Self { config }
    }

    pub fn gemini_api_key(&self) -> Result<String> {
        if let Ok(value) = std::env::var(&self.config.secrets.env_var) {
            if !value.trim().is_empty() {
                return Ok(value);
            }
        }

        if !self.config.service.gemini_api_key.trim().is_empty() {
            return Ok(self.config.service.gemini_api_key.clone());
        }

        let entry = Entry::new(
            &self.config.secrets.keyring_service,
            &self.config.secrets.keyring_account,
        )
        .map_err(|err| InternalVoiceError::Secret(self.gemini_lookup_failure(Some(err.to_string()))))?;
        entry.get_password().map_err(|err| {
            InternalVoiceError::Secret(self.gemini_lookup_failure(Some(err.to_string())))
        })
    }

    fn gemini_lookup_failure(&self, keyring_error: Option<String>) -> String {
        let mut message = format!(
            "Gemini API key not found. Tried env var `{}`, config field `service.gemini_api_key`, and keyring service `{}` account `{}`",
            self.config.secrets.env_var,
            self.config.secrets.keyring_service,
            self.config.secrets.keyring_account
        );

        if let Some(err) = keyring_error {
            message.push_str(&format!("; keyring error: {err}"));
        }

        message
    }
}

pub fn enforce_least_privilege() {
    #[cfg(unix)]
    {
        let uid = unsafe { libc::geteuid() };
        if uid == 0 {
            tracing::warn!("service is running as root; deploy with a restricted service user");
        }
    }

    #[cfg(windows)]
    {
        tracing::info!("verify the Windows service account has restricted privileges");
    }
}

pub fn validate_action(action: &AllowedAction) -> bool {
    matches!(
        action,
        AllowedAction::Announce
            | AllowedAction::QuerySystemState
            | AllowedAction::SummarizeTopProcesses
    )
}

pub fn validate_config(config: &AppConfig) -> Result<()> {
    if config.limits.requests_per_minute == 0 {
        return Err(InternalVoiceError::Config(
            "limits.requests_per_minute must be > 0".into(),
        ));
    }

    if config.service.gemini_model.trim().is_empty() {
        return Err(InternalVoiceError::Config(
            "service.gemini_model cannot be empty".into(),
        ));
    }

    Ok(())
}
