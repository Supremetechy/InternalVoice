use std::{
    path::Path,
    sync::Mutex,
    time::{Duration, UNIX_EPOCH},
};

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::error::Result;

pub struct PromptCache {
    conn: Mutex<Connection>,
    ttl: Duration,
}

impl PromptCache {
    pub fn open(path: &Path, ttl: Duration) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS prompt_cache (
                key TEXT PRIMARY KEY,
                response TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            ttl,
        })
    }

    pub fn get(&self, prompt: &str) -> Result<Option<String>> {
        let key = hash(prompt);
        let cutoff = now_secs().saturating_sub(self.ttl.as_secs());
        let conn = self.conn.lock().expect("cache mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT response FROM prompt_cache
             WHERE key = ?1 AND created_at >= ?2",
        )?;

        let mut rows = stmt.query(params![key, cutoff])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(row.get(0)?));
        }

        Ok(None)
    }

    pub fn put(&self, prompt: &str, response: &str) -> Result<()> {
        let key = hash(prompt);
        let created_at = now_secs();
        let conn = self.conn.lock().expect("cache mutex poisoned");
        conn.execute(
            "INSERT INTO prompt_cache(key, response, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
                response = excluded.response,
                created_at = excluded.created_at",
            params![key, response, created_at],
        )?;
        Ok(())
    }
}

fn hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_secs() -> u64 {
    UNIX_EPOCH.elapsed().unwrap_or_default().as_secs()
}

