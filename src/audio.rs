use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::{OutputStream, Sink, buffer::SamplesBuffer};
use base64::{Engine as _, engine::general_purpose};
use tokio::sync::mpsc;

use crate::error::{InternalVoiceError, Result};

enum PlaybackMsg {
    Samples(Vec<i16>),
    Clear,
}

pub struct AudioEngine {
    input_device: cpal::Device,
    input_config: cpal::SupportedStreamConfig,
    /// Rate at which microphone audio is encoded before sending to Gemini (16 kHz PCM).
    encode_rate: u32,
    /// Native sample rate of the hardware output device.
    output_native_rate: u32,
    /// Cached name of the output device (device is moved into the playback thread).
    output_device_name: Option<String>,
    /// Send audio commands to the persistent playback thread.
    playback_tx: std::sync::mpsc::Sender<PlaybackMsg>,
    /// True while the Sink has audio queued or playing; mic is muted during this time.
    is_playing: Arc<AtomicBool>,
    /// Set on turn_complete; causes the next play_audio call to flush stale audio first.
    pending_clear: Arc<AtomicBool>,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();

        let input_device = host
            .default_input_device()
            .ok_or_else(|| InternalVoiceError::Audio("No input device found".into()))?;
        let output_device = host
            .default_output_device()
            .ok_or_else(|| InternalVoiceError::Audio("No output device found".into()))?;

        let input_config = input_device
            .default_input_config()
            .map_err(|e| InternalVoiceError::Audio(format!("Failed to get default input config: {}", e)))?;

        let output_native_rate = output_device
            .default_output_config()
            .map(|c| c.sample_rate().0)
            .unwrap_or(44100);

        let output_device_name = output_device.name().ok();

        tracing::debug!(
            "Input: channels={}, sample_rate={}, format={:?} | Output native rate: {}",
            input_config.channels(),
            input_config.sample_rate().0,
            input_config.sample_format(),
            output_native_rate,
        );

        let is_playing = Arc::new(AtomicBool::new(false));
        let pending_clear = Arc::new(AtomicBool::new(false));
        let (playback_tx, playback_rx) = std::sync::mpsc::channel::<PlaybackMsg>();

        let native_rate = output_native_rate;
        let is_playing_thread = Arc::clone(&is_playing);
        std::thread::spawn(move || {
            let (_stream, handle) = match OutputStream::try_from_device(&output_device) {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::error!("Failed to open audio output device: {}", e);
                    return;
                }
            };
            let sink = match Sink::try_new(&handle) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to create audio sink: {}", e);
                    return;
                }
            };

            while let Ok(msg) = playback_rx.recv() {
                match msg {
                    PlaybackMsg::Samples(samples) => {
                        is_playing_thread.store(true, Ordering::Release);
                        sink.append(SamplesBuffer::new(1, native_rate, samples));

                        // Eagerly drain any additional samples that are already queued.
                        let mut cleared = false;
                        loop {
                            match playback_rx.try_recv() {
                                Ok(PlaybackMsg::Samples(s)) => {
                                    sink.append(SamplesBuffer::new(1, native_rate, s));
                                }
                                Ok(PlaybackMsg::Clear) => {
                                    sink.clear();
                                    is_playing_thread.store(false, Ordering::Release);
                                    cleared = true;
                                    break;
                                }
                                Err(_) => break,
                            }
                        }
                        if cleared {
                            continue;
                        }

                        // Poll until the Sink fully drains, accepting new messages along the way.
                        loop {
                            if sink.empty() {
                                is_playing_thread.store(false, Ordering::Release);
                                break;
                            }
                            match playback_rx.try_recv() {
                                Ok(PlaybackMsg::Samples(s)) => {
                                    sink.append(SamplesBuffer::new(1, native_rate, s));
                                }
                                Ok(PlaybackMsg::Clear) => {
                                    sink.clear();
                                    is_playing_thread.store(false, Ordering::Release);
                                    break;
                                }
                                Err(_) => {}
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                    PlaybackMsg::Clear => {
                        sink.clear();
                        is_playing_thread.store(false, Ordering::Release);
                    }
                }
            }
            sink.sleep_until_end();
        });

        Ok(Self {
            input_device,
            input_config,
            encode_rate: 16000,
            output_native_rate,
            output_device_name,
            playback_tx,
            is_playing,
            pending_clear,
        })
    }

    pub fn output_sample_rate(&self) -> u32 {
        self.output_native_rate
    }

    pub fn input_device_name(&self) -> Option<String> {
        self.input_device.name().ok()
    }

    pub fn output_device_name(&self) -> Option<String> {
        self.output_device_name.clone()
    }

    /// Called when Gemini signals turn_complete. The next audio chunk will flush any
    /// leftover samples from the previous turn before enqueuing new ones.
    pub fn notify_turn_complete(&self) {
        self.pending_clear.store(true, Ordering::Release);
    }

    pub fn start_recording(&self, tx: mpsc::Sender<String>) -> Result<cpal::Stream> {
        let sample_rate = self.input_config.sample_rate().0;
        let channels = self.input_config.channels();
        let target_sample_rate = self.encode_rate;
        let is_playing = Arc::clone(&self.is_playing);

        let stream = match self.input_config.sample_format() {
            cpal::SampleFormat::I16 => self.build_input_stream::<i16>(
                tx, channels, sample_rate, target_sample_rate, is_playing,
            )?,
            cpal::SampleFormat::F32 => self.build_input_stream::<f32>(
                tx, channels, sample_rate, target_sample_rate, is_playing,
            )?,
            format => {
                return Err(InternalVoiceError::Audio(format!(
                    "Unsupported sample format: {:?}",
                    format
                )))
            }
        };

        stream
            .play()
            .map_err(|e: cpal::PlayStreamError| InternalVoiceError::Audio(e.to_string()))?;
        Ok(stream)
    }

    fn build_input_stream<T>(
        &self,
        tx: mpsc::Sender<String>,
        channels: u16,
        source_rate: u32,
        target_rate: u32,
        is_playing: Arc<AtomicBool>,
    ) -> Result<cpal::Stream>
    where
        T: cpal::Sample + rodio::Sample + Into<f32> + cpal::SizedSample,
    {
        let tx = tx.clone();
        self.input_device
            .build_input_stream(
                &self.input_config.clone().into(),
                move |data: &[T], _| {
                    // Mute mic while Gemini is speaking to prevent acoustic echo feedback.
                    if is_playing.load(Ordering::Acquire) {
                        return;
                    }

                    let mut samples: Vec<f32> = data.iter().map(|&s| s.into()).collect();

                    if channels > 1 {
                        let mut mono = Vec::with_capacity(samples.len() / channels as usize);
                        for chunk in samples.chunks_exact(channels as usize) {
                            let avg = chunk.iter().sum::<f32>() / channels as f32;
                            mono.push(avg);
                        }
                        samples = mono;
                    }

                    let final_samples = if source_rate != target_rate {
                        resample(&samples, source_rate, target_rate)
                    } else {
                        samples
                    };

                    let pcm_data: Vec<u8> = final_samples
                        .iter()
                        .flat_map(|&s| {
                            let sample = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            sample.to_le_bytes()
                        })
                        .collect();

                    let encoded = general_purpose::STANDARD.encode(&pcm_data);
                    let _ = tx.blocking_send(encoded);
                },
                |err| tracing::error!("Audio input stream error: {}", err),
                None,
            )
            .map_err(|e| InternalVoiceError::Audio(e.to_string()))
    }

    /// Decode base64 PCM audio from Gemini, resample to the device's native rate,
    /// and enqueue it on the persistent Sink. Returns immediately — no blocking.
    pub fn play_audio(&self, base64_data: &str, source_rate: u32) -> Result<()> {
        let decoded = general_purpose::STANDARD
            .decode(base64_data)
            .map_err(|e| InternalVoiceError::Audio(format!("Base64 decode error: {}", e)))?;

        // Ignore sub-64-byte packets — these are marker/padding frames, not real audio.
        if decoded.len() < 64 {
            return Ok(());
        }

        // If a new turn just started, flush any stale audio from the previous turn.
        if self.pending_clear.swap(false, Ordering::AcqRel) {
            let _ = self.playback_tx.send(PlaybackMsg::Clear);
        }

        let f32_samples: Vec<f32> = decoded
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
            .collect();

        let resampled = resample(&f32_samples, source_rate, self.output_native_rate);

        let samples: Vec<i16> = resampled
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();

        tracing::debug!(
            "Enqueuing {} mono samples ({}Hz → {}Hz)",
            samples.len(),
            source_rate,
            self.output_native_rate
        );

        self.playback_tx
            .send(PlaybackMsg::Samples(samples))
            .map_err(|_| InternalVoiceError::Audio("Playback thread has exited".into()))?;

        Ok(())
    }
}

fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return samples.to_vec();
    }

    let ratio = from as f32 / to as f32;
    let target_len = (samples.len() as f32 / ratio).floor() as usize;
    let mut result = Vec::with_capacity(target_len);

    for i in 0..target_len {
        let pos = i as f32 * ratio;
        let idx = pos.floor() as usize;
        let frac = pos - idx as f32;

        if idx + 1 < samples.len() {
            result.push(samples[idx] * (1.0 - frac) + samples[idx + 1] * frac);
        } else {
            result.push(samples[idx]);
        }
    }

    result
}
