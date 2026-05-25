use std::{fs, path::PathBuf};

use crate::{error::Result, state::model::SystemState};

pub struct StatePublisher {
    current_state_path: PathBuf,
}

impl StatePublisher {
    pub fn new(current_state_path: PathBuf) -> Self {
        Self { current_state_path }
    }

    pub fn publish(&self, state: &SystemState) -> Result<()> {
        let payload = serde_json::to_vec_pretty(state)?;
        fs::write(&self.current_state_path, payload)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &PathBuf {
        &self.current_state_path
    }
}

