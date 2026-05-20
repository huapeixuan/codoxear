# Contract harness

Phase 6 is Rust-only. The retained contract tests start `codoxear-backend-rs`
and exercise public HTTP/disk contracts directly; they do not start or import a
Python backend.

Build first, then run:

```bash
cargo build --manifest-path backend-rs/Cargo.toml --release --bins
pytest tests/contract -q
```

The suite covers canonical `/api/v1/*` and legacy `/api/*` aliases, pre-cutover
sidecar readability, static/file endpoint behavior, auth-cookie compatibility,
and unknown-route boundaries.
