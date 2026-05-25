use tokio::{process::Command, sync::mpsc};
use tracing::warn;

/// Lightweight TTS engine backed by the platform's native speech facility.
/// Utterances are queued and spoken sequentially so they never overlap.
pub struct TtsEngine {
    sender: mpsc::Sender<String>,
}

impl TtsEngine {
    pub fn new() -> Self {
        let (sender, mut receiver) = mpsc::channel::<String>(32);
        tokio::spawn(async move {
            while let Some(text) = receiver.recv().await {
                speak_platform(&text).await;
            }
        });
        Self { sender }
    }

    /// Queue `text` for speech. Returns immediately; speech runs in background.
    pub async fn speak(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Err(e) = self.sender.send(text.to_string()).await {
            warn!(error = %e, "tts: channel closed; utterance dropped");
        }
    }
}

async fn speak_platform(text: &str) {
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = Command::new("say").arg(text).status().await {
            warn!(error = %e, "tts: say command failed");
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Replace straight quotes to avoid breaking the PowerShell string literal.
        let safe = text.replace('"', "\u{201C}").replace('\'', "\u{2018}");
        let script = format!(
            "Add-Type -AssemblyName System.Speech; \
             (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak(\"{}\")",
            safe
        );
        if let Err(e) = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .await
        {
            warn!(error = %e, "tts: powershell speak failed");
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        use tokio::io::AsyncWriteExt as _;

        // Prefer espeak; fall back to festival via stdin.
        if Command::new("espeak").arg(text).status().await.is_ok() {
            return;
        }
        match Command::new("festival")
            .arg("--tts")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes()).await;
                }
                let _ = child.wait().await;
            }
            Err(_) => warn!("tts: no speech engine found; install espeak or festival"),
        }
    }
}
