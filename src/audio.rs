use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::{OutputStream, Sink, buffer::SamplesBuffer};
use base64::{Engine as _, engine::general_purpose};
use tokio::sync::mpsc;

use crate::error::{InternalVoiceError, Result};

pub struct AudioEngine {
    input_device: cpal::Device,
    output_device: cpal::Device,
    sample_rate: u32,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let input_device = host.default_input_device()
            .ok_or_else(|| InternalVoiceError::Audio("No input device found".into()))?;
        let output_device = host.default_output_device()
            .ok_or_else(|| InternalVoiceError::Audio("No output device found".into()))?;

        Ok(Self {
            input_device,
            output_device,
            sample_rate: 16000,
        })
    }

    pub fn start_recording(&self, tx: mpsc::Sender<String>) -> Result<cpal::Stream> {
        let config = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(self.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = self.input_device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                // Convert PCM i16 to bytes and then Base64
                let bytes: Vec<u8> = data.iter()
                    .flat_map(|&sample| sample.to_le_bytes())
                    .collect();
                
                let encoded = general_purpose::STANDARD.encode(&bytes);
                let _ = tx.blocking_send(encoded);
            },
            |err| tracing::error!("Audio input stream error: {}", err),
            None
        ).map_err(|e| InternalVoiceError::Audio(e.to_string()))?;

        stream.play().map_err(|e: cpal::PlayStreamError| InternalVoiceError::Audio(e.to_string()))?;
        Ok(stream)
    }

    pub fn play_audio(&self, base64_data: &str) -> Result<()> {
        let decoded = general_purpose::STANDARD.decode(base64_data)
            .map_err(|e| InternalVoiceError::Audio(format!("Base64 decode error: {}", e)))?;

        // Convert bytes back to i16 samples
        let samples: Vec<i16> = decoded.chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        // Play using rodio
        let (_stream, handle) = OutputStream::try_from_device(&self.output_device)
            .map_err(|e| InternalVoiceError::Audio(e.to_string()))?;
        let sink = Sink::try_new(&handle)
            .map_err(|e| InternalVoiceError::Audio(e.to_string()))?;

        let buffer = SamplesBuffer::new(1, self.sample_rate, samples);
        sink.append(buffer);
        sink.sleep_until_end();

        Ok(())
    }
}
