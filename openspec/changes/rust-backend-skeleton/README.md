# rust-backend-skeleton

Phase 1 of `rust-backend-cutover`: stand up the `backend-rs/` Rust crate (axum 0.7 + tokio) with three read-only endpoints — `/api/health`, `/api/me`, `/api/sessions/bootstrap` — and the canonical `/api/v1/...` aliases, behind a runtime that can be left disabled for instant rollback to the Python backend.
