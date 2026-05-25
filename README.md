# InternalVoice

`InternalVoice` is a Rust-based cross-platform systems utility that samples local system health, normalizes it into a structured state model, and generates concise spoken or textual system narratives through Gemini.

## Current status

This repository now contains a build-oriented service scaffold:

- Tokio async service loop with graceful shutdown on `SIGTERM` or `Ctrl+C`
- Cross-platform system sampling via `sysinfo`
- Normalized `SystemState` model published to JSON
- Policy engine that only narrates actionable conditions
- Gemini client wrapper with HTTP reuse, streaming responses, timeout, rate limiting, prompt cache, and circuit breaker behavior
- Secret lookup via environment variable first, then OS-native keyring
- Least-privilege and allow-list guardrails around model-driven actions

## Important model note

As of March 28, 2026, the Google AI for Developers Gemini API model guide lists `gemini-3-flash-preview` as the stable Flash-Lite model for text generation via `generateContent`. The checked source was:

- https://ai.google.dev/gemini-api/docs/models/gemini

This scaffold uses the Gemini Developer API `v1beta` `streamGenerateContent` endpoint, so the default config now uses `gemini-2.5-flash-lite`. If you want native low-latency audio sessions next, the right follow-up is a WebSocket or bidirectional streaming transport module using a Live API-capable model instead of the text-generation endpoint.

## Layout

- `/InternalVoice/src/main.rs`: service lifecycle and loop
- `/InternalVoice/src/context.rs`: shared `ServiceContext`
- `/InternalVoice/src/sensors/system.rs`: metric collection
- `/InternalVoice/src/state/model.rs`: normalized state schema
- `/InternalVoice/src/state/sampler.rs`: alert derivation
- `InternalVoice/src/policy.rs`: narration policy and debounce boundary
- `InternalVoice/src/gemini.rs`: Gemini transport, rate limiting, and breaker
- `/InternalVoice/src/security.rs`: secret resolution and allow-list checks
- `/InternalVoice/src/publisher.rs`: `current-state.json` publication
- `/InternalVoice/config/InternalVoice.example.toml`: example configuration

## Configuration

Set a config path explicitly:

```bash
export INTERNALVOICE_CONFIG=/path/to/config.toml
```

If `INTERNALVOICE_CONFIG` is unset or points to a missing file, the service falls back to `./config.toml` and then `./config/InternalVoice.example.toml`.

API key resolution order:

1. `.env` or process environment variable from `[secrets].env_var`
2. Optional `[service].gemini_api_key` value in the TOML
3. OS keyring entry from `[secrets].keyring_service` and `[secrets].keyring_account`

Config defaults if omitted:

- `service.gemini_api_key = ""`
- `service.log_dir = "logs"`
- `limits.system_telemetry_interval_secs = 60`

## Next implementation steps

- Add platform adapters for macOS `IOKit`, Windows `PDH/WMI`, and Linux `/proc`, `sysfs`, and vendor GPU APIs
- Replace the placeholder battery implementation with platform-native power telemetry
- Add a true Gemini Live API transport for duplex voice interaction
- Add STT/TTS providers behind traits so the core service stays platform-neutral
- Add per-platform installers and service definitions for `systemd`, `launchd`, and Windows Service Control Manager

## Build

Rust tooling is not installed in the current environment, so the crate could not be compiled here yet. Once `cargo` is available:

```bash
cargo check
cargo run
```
