use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::{OutputStream, Sink, buffer::SamplesBuffer};
use base64::{Engine as _, engine::general_purpose};
use tokio::sync::mpsc;

use crate::error::{InternalVoiceError, Result};

pub struct AudioEngine {
    input_device: cpal::Device,
    output_device: cpal::Device,
    input_config: cpal::SupportedStreamConfig,
    /// Rate used when encoding microphone audio to send to Gemini (16 kHz PCM).
    encode_rate: u32,
    /// Native sample rate of the hardware output device.
    output_native_rate: u32,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let input_device = host.default_input_device()
            .ok_or_else(|| InternalVoiceError::Audio("No input device found".into()))?;
        let output_device = host.default_output_device()
            .ok_or_else(|| InternalVoiceError::Audio("No output device found".into()))?;

        let input_config = input_device.default_input_config()
            .map_err(|e| InternalVoiceError::Audio(format!("Failed to get default input config: {}", e)))?;

        let output_native_rate = output_device
            .default_output_config()
            .map(|c| c.sample_rate().0)
            .unwrap_or(44100);

        tracing::debug!(
            "Input: channels={}, sample_rate={}, format={:?} | Output native rate: {}",
            input_config.channels(),
            input_config.sample_rate().0,
            input_config.sample_format(),
            output_native_rate,
        );

        Ok(Self {
            input_device,
            output_device,
            input_config,
            encode_rate: 16000,
            output_native_rate,
        })
    }

    /// The hardware output device's native sample rate. Pass this to Gemini Live
    /// so it can target the same rate and no resampling is required on playback.
    pub fn output_sample_rate(&self) -> u32 {
        self.output_native_rate
    }

    pub fn input_device_name(&self) -> Option<String> {
        self.input_device.name().ok()
    }

    pub fn output_device_name(&self) -> Option<String> {
        self.output_device.name().ok()
    }

    pub fn start_recording(&self, tx: mpsc::Sender<String>) -> Result<cpal::Stream> {
        let sample_rate = self.input_config.sample_rate().0;
        let channels = self.input_config.channels();
        let target_sample_rate = self.encode_rate;

        let stream = match self.input_config.sample_format() {
            cpal::SampleFormat::I16 => self.build_input_stream::<i16>(tx, channels, sample_rate, target_sample_rate)?,
            cpal::SampleFormat::F32 => self.build_input_stream::<f32>(tx, channels, sample_rate, target_sample_rate)?,
            format => return Err(InternalVoiceError::Audio(format!("Unsupported sample format: {:?}", format))),
        };

        stream.play().map_err(|e: cpal::PlayStreamError| InternalVoiceError::Audio(e.to_string()))?;
        Ok(stream)
    }

    fn build_input_stream<T>(&self, tx: mpsc::Sender<String>, channels: u16, source_rate: u32, target_rate: u32) -> Result<cpal::Stream>
    where
        T: cpal::Sample + rodio::Sample + Into<f32> + cpal::SizedSample,
    {
        let tx = tx.clone();
        self.input_device.build_input_stream(
            &self.input_config.clone().into(),
            move |data: &[T], _| {
                let mut samples: Vec<f32> = data.iter().map(|&s| s.into()).collect();
                
                // Convert to mono if multi-channel
                if channels > 1 {
                    let mut mono = Vec::with_capacity(samples.len() / channels as usize);
                    for chunk in samples.chunks_exact(channels as usize) {
                        let avg = chunk.iter().sum::<f32>() / channels as f32;
                        mono.push(avg);
                    }
                    samples = mono;
                }

                // Resample if necessary
                let final_samples = if source_rate != target_rate {
                    resample(&samples, source_rate, target_rate)
                } else {
                    samples
                };

                // Convert f32 to i16 PCM
                let pcm_data: Vec<u8> = final_samples.iter()
                    .flat_map(|&s| {
                        let sample = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        sample.to_le_bytes()
                    })
                    .collect();

                let encoded = general_purpose::STANDARD.encode(&pcm_data);
                let _ = tx.blocking_send(encoded);
            },
            |err| tracing::error!("Audio input stream error: {}", err),
            None
        ).map_err(|e| InternalVoiceError::Audio(e.to_string()))
    }

    /// Play base64-encoded PCM audio that Gemini sent at `source_rate`.
    /// The samples are resampled to the hardware's native rate so rodio
    /// receives audio that already matches the device — no implicit resampling.
    pub fn play_audio(&self, base64_audio: &str, source_rate: u32) -> Result<()> {
        let decoded = general_purpose::STANDARD.decode(base64_audio)
            .map_err(|e| InternalVoiceError::Audio(format!("Base64 decode error: {}", e)))?;

        // Decode i16 LE PCM → f32 for resampling.
        let f32_samples: Vec<f32> = decoded
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
            .collect();

        // Resample from Gemini's rate to the hardware's native rate.
        let resampled = resample(&f32_samples, source_rate, self.output_native_rate);

        // Convert back to i16 for rodio.
        let i16_samples: Vec<i16> = resampled
            .into_iter()
            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();

        let (_stream, handle) = OutputStream::try_from_device(&self.output_device)
            .map_err(|e| InternalVoiceError::Audio(e.to_string()))?;
        let sink = Sink::try_new(&handle)
            .map_err(|e| InternalVoiceError::Audio(e.to_string()))?;

        // Tell rodio the buffer is already at the device's native rate.
        let buffer = SamplesBuffer::new(1, self.output_native_rate, i16_samples);
        sink.append(buffer);
        sink.sleep_until_end();

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
            let sample = samples[idx] * (1.0 - frac) + samples[idx + 1] * frac;
            result.push(sample);
        } else {
            result.push(samples[idx]);
        }
    }

    result
}
