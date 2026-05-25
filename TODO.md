# TODO - InternalVoice (Gemini Live 404 fix)

- [x] Inspect Gemini Live connect path and identify hard-coded WSS endpoint causing 404.
- [x] Add config override `service.gemini_live_ws_url` to allow changing the Live WebSocket endpoint.
- [x] Update Gemini Live client to use `gemini_live_ws_url` template with `{key}`.
- [x] Improve Gemini Live connect error logging with redacted URL.
- [x] Update `config/InternalVoice.example.toml` to document the new `gemini_live_ws_url` option.

- [x] Build/check the project (`cargo check`).

- [ ] Run and verify that Live fails gracefully and polling continues.


