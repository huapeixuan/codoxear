# Contract parity harness

This directory contains cross-backend parity tests for the Rust cutover. Phase 0
only adds the harness skeleton; endpoint tests are intentionally skipped until
Phase 1 provides a Rust binary.

## Current Phase 0 smoke check

```bash
pytest tests/contract
```

Expected result in Phase 0: tests collect cleanly and skip because
`rust_server_url` is not implemented yet.

## Running once Rust is available

Build the Rust backend binaries first:

```bash
cargo build --release --bins -C backend-rs
pytest tests/contract -k parity
```

If your Cargo version does not accept `-C`, run the equivalent from the crate
directory:

```bash
(cd backend-rs && cargo build --release --bins)
pytest tests/contract -k parity
```

Expected environment:

- `CODEX_WEB_PASSWORD` is set by the fixtures.
- `CODEX_WEB_HOST=127.0.0.1` and random `CODEX_WEB_PORT` are set by the fixtures.
- The Python fixture isolates runtime state by setting `HOME` to a temporary
  directory, yielding a temporary `~/.local/share/codoxear` tree.
- Phase 1 should make the Rust fixture use an equivalent temporary app dir / home
  and point it at `backend-rs/target/release/codoxear-backend-rs`.

Common fail modes:

- Collection error: the harness skeleton has drifted from pytest discovery.
- Python fixture startup timeout: `CODEX_WEB_PASSWORD`, static asset mode, or port
  binding regressed.
- Rust fixture skip: expected until Phase 1.
- JSON diff: compare the failing key against `docs/cutover/endpoint-inventory.md`
  and `docs/cutover/disk-contracts.md`; add an ignore key only for documented
  volatile values such as `app_version`.
