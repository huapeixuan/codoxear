# rust-backend-readonly-routes

Phase 2 of `rust-backend-cutover`: extend the `backend-rs/` Rust crate from Phase 1's three readonly endpoints (`/api/health`, `/api/me`, `/api/sessions/bootstrap`) to the **full set of GET / readonly endpoints** in `docs/cutover/endpoint-inventory.md`, while folding in the Phase 1 review退回项 (Content-Type charset parity, `serde_json::preserve_order`, `cwd_groups` error recovery, `normalize_cwd_group_key` progressive canonical, populated-state bootstrap parity tests).

POST writes (Phase 3), broker port (Phase 4), HLS / WebPush / TTS workers (Phase 5), and `/api/audio/*` are explicitly out of scope.
