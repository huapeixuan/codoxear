## ADDED Requirements

### Requirement: Atomic JSON runtime state writes
Codoxear server runtime JSON state files SHALL be written through a single internal atomic persistence helper so repeated state-saving code paths preserve consistent encoding, directory creation, temporary-file, replacement, and cleanup semantics.

#### Scenario: Persisting a runtime state file
- **WHEN** a server runtime JSON state object is saved
- **THEN** the system MUST write UTF-8 JSON with the expected indentation and trailing newline through a temporary file before atomically replacing the target path

#### Scenario: Failed temporary write
- **WHEN** writing or replacing a runtime JSON state file fails after a temporary path is chosen
- **THEN** the system MUST attempt to remove the temporary file before propagating the original error

### Requirement: Runtime state schema compatibility
The refactor MUST preserve the existing JSON data shapes and public API behavior for session harness, aliases, sidebar metadata, hidden sessions, file history, queues, recent working directories, and working-directory groups.

#### Scenario: Existing state save methods
- **WHEN** an existing session state save method persists one of the supported runtime state files
- **THEN** the resulting JSON structure MUST remain compatible with the corresponding existing load method
