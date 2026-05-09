# Contract parity harness

This directory contains cross-backend parity tests for the Rust cutover. Phase 1
runs the Python server and the new Rust backend against a shared temporary HOME
so both processes see the same `~/.local/share/codoxear` state and `hmac_secret`.

## Parity checks

Build the Rust backend binaries first:

```bash
(cd backend-rs && cargo build --release --bins)
pytest tests/contract -q -k 'parity and not readonly'
pytest tests/contract -q -k 'parity and readonly'
```

Phase 2 readonly parity requires `gh` on `PATH` (macOS CI installs it with
`brew install gh`) so git-context tests exercise the no-auth/no-PR paths instead
of skipping due to a missing CLI.

Current implementation status: Phase 1 retest covers `/api/me` and
`/api/sessions/bootstrap` including populated bootstrap state; the broader
readonly endpoint tests are unlocked wave-by-wave by
`rust-backend-readonly-routes`.

If you need to run Python-only contract collection without the Rust binary, set:

```bash
CODOXEAR_SKIP_RUST_FIXTURE=1 pytest tests/contract -q -k parity
```

Without that escape hatch, a missing Rust binary is a fixture error with a hint to
run `(cd backend-rs && cargo build --release --bins)`.

## Fixtures

- `shared_app_home`: session-scoped temporary HOME shared by Python and Rust.
- `python_server_url`: starts `python -m codoxear.server` on a random localhost
  port with `CODEX_WEB_PASSWORD`, `CODEX_WEB_HOST=127.0.0.1`, and the shared HOME.
- `rust_server_url`: starts `backend-rs/target/release/codoxear-backend-rs` on a
  random localhost port with the same password and HOME.
- `signed_auth_cookie`: lazily creates/loads
  `<shared_app_home>/.local/share/codoxear/hmac_secret` and signs a
  `codoxear_auth` cookie directly. Phase 1 does this because `/api/login` is out
  of scope for the Rust skeleton.

Common fail modes:

- Python fixture startup timeout: `CODEX_WEB_PASSWORD`, static asset mode, or port
  binding regressed.
- Rust fixture error: build the release binaries, or set
  `CODOXEAR_SKIP_RUST_FIXTURE=1` for Python-only runs.
- JSON diff: compare the failing key against `docs/cutover/endpoint-inventory.md`
  and `docs/cutover/disk-contracts.md`; add an ignore key only for documented
  volatile values such as `app_version`.
