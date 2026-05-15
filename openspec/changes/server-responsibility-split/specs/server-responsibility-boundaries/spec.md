## ADDED Requirements

### Requirement: Public HTTP contracts remain stable during refactor
The refactor SHALL preserve existing public HTTP API behavior for all routes moved out of `codoxear/server.py`.

#### Scenario: Session list routes keep response semantics
- **WHEN** a client calls `GET /api/sessions`, `GET /api/sessions?view=directories`, `GET /api/sessions?view=recent`, or `GET /api/sessions/bootstrap` with valid authentication
- **THEN** the response MUST keep the same status code, pagination semantics, and public JSON fields as before the refactor

#### Scenario: Workspace and file routes keep response semantics
- **WHEN** a client calls existing workspace, file-list, file-read, file-blob, file-inspect, or git-file-version routes with valid inputs
- **THEN** the response MUST keep the same success shape, error status codes, and path-safety behavior as before the refactor

#### Scenario: Session diagnostics and queue routes keep response semantics
- **WHEN** a client calls existing diagnostics, live, workspace, details, commands, queue, or repo-context routes
- **THEN** the response MUST keep the same public fields and MUST NOT expose additional internal manager fields by accident

### Requirement: Service modules have single-purpose boundaries
The refactored server code SHALL split large responsibilities into focused modules without introducing circular ownership or duplicated semantics.

#### Scenario: Session list projection is isolated
- **WHEN** code builds the frontend session-list or recent-session payload
- **THEN** pagination, grouping, hidden-group filtering, recency ordering, and frontend row projection MUST live in a dedicated session-list module or equivalent focused boundary instead of being embedded in the HTTP handler

#### Scenario: Workspace file operations are isolated
- **WHEN** code resolves session-relative file paths, lists a directory, reads file content, inspects file metadata, serves file blobs, or reads git file versions
- **THEN** path validation and file operation helpers MUST live in a dedicated workspace/file module or equivalent focused boundary and MUST be reused by the routes

#### Scenario: HTTP handlers delegate to service helpers
- **WHEN** the HTTP handler receives a request for a moved route
- **THEN** it MUST limit itself to authentication, query/body parsing, error-to-response mapping, and calling focused helpers; it MUST NOT reimplement business logic inline

### Requirement: SessionManager state persistence remains compatible
The refactor SHALL preserve the existing on-disk runtime state files and lifecycle side effects managed by `SessionManager`.

#### Scenario: Existing runtime files are read without migration
- **WHEN** the server starts with existing `session_sidebar.json`, `session_files.json`, `session_queues.json`, `harness.json`, `session_aliases.json`, `recent_cwds.json`, or `cwd_groups.json`
- **THEN** the refactored code MUST read them without requiring a manual migration or data reset

#### Scenario: State updates preserve existing file shapes
- **WHEN** aliases, queues, cwd groups, recent cwd entries, harness config, hidden sessions, sidebar metadata, or session file history are updated
- **THEN** the persisted JSON shape MUST remain backward-compatible with the current implementation

#### Scenario: Dead-session cleanup preserves side effects
- **WHEN** a session is pruned, deleted, hidden, or refreshed
- **THEN** the same persisted state cleanup and queue-drain side effects MUST occur as before the refactor

### Requirement: Refactor is guarded by targeted regression tests
The refactor SHALL include tests that prove behavior preservation at both module and route level.

#### Scenario: Module extraction has focused unit tests
- **WHEN** session-list projection or workspace-file helpers are moved into new modules
- **THEN** those modules MUST have focused tests covering normal, boundary, and error cases without requiring a live broker process

#### Scenario: Route contracts are regression-tested
- **WHEN** HTTP routes are rewired to call extracted helpers
- **THEN** existing route tests MUST pass and new tests MUST cover any route whose handler logic is materially moved

#### Scenario: Full verification is run before handoff
- **WHEN** coding completes this change
- **THEN** `python3 -m pytest` for the touched backend suites and `cd web && npm run build` MUST pass before the issue is moved to review
