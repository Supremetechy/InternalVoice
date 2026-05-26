# TODO - InternalVoice (Gemini Live fix)

Inspect Gemini Live connect path and identify hard-coded WSS endpoint causing 404.
Add config override `service.gemini_live_ws_url` to allow changing the Live WebSocket endpoint.
Update Gemini Live client to use `gemini_live_ws_url` template with `{key}`.
Update `config/InternalVoice.example.toml` to document the new `gemini_live_ws_url` option.

Build/check the project (`cargo check`).



