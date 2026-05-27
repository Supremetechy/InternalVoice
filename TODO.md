# InternalVoice - TODO

## Phase 1: Gemini Live tool-response schema fix
- [ ] Inspect current Gemini Live tool-response structs and message composition.
- [ ] Update `src/gemini.rs` tool-response JSON schema to match Gemini Live expectations (fix unknown `response`).
- [ ] Add debug logging for the outgoing tool response payload.
- [ ] Run `cargo check` to ensure compilation.
- [ ] (Optional) Run the service to reproduce the prior disconnect and confirm the fix.

