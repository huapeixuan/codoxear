# Workspace File Tree Performance Design

Date: 2026-04-16

## Goal

Make workspace directory expand/collapse interactions feel fast in the file viewer, especially after a directory has already been loaded once.

## User Intent

The current workspace file browser feels slow when expanding directories. The user reports:
- first expand is slow
- re-expanding the same directory is faster than the first time
- but even repeated expand/collapse still feels slow

The change should improve both:
- actual work done during expand/collapse
- perceived responsiveness of the file viewer

## Current State

The file browser lives in `web/src/components/workspace/FileViewerDialog.tsx`.

Today it stores the file tree as nested `TreeNode[]` state and updates it with recursive whole-tree helpers:
- `mapTreeNodes()`
- `setTreeNodeExpanded()`
- `setTreeNodeLoading()`
- `setTreeNodeError()`
- `mergeTreeChildren()`

When toggling a directory, the component currently does two expensive things:
1. recursively walks the loaded tree to update the target node
2. recursively walks the tree again to find the target node and decide whether it must fetch children

Because `FileTreeNodeRow` is also rendered recursively without memoization, toggling one loaded directory can trigger broad rerendering across the visible tree.

This means the first expand cost is a combination of:
- network request for `/api/sessions/:id/file/list?path=<dir>`
- React/Preact state update work across the existing tree
- rerender work for unaffected rows

The repeated expand cost drops because children are cached, but it still pays too much local state-update and rerender cost.

## Recommended Direction

Replace the nested-tree mutation model with a path-indexed tree store local to `FileViewerDialog`, then render rows from indexed node records.

This keeps lazy loading intact but makes expand/collapse updates local instead of full-tree recursive operations.

## Design Overview

### 1. State Model

Replace:
- `tree: TreeNode[]`

With local state shaped conceptually like:
- `rootPaths: string[]`
- `nodesByPath: Record<string, TreeNodeRecord>`

Each node record should hold:
- `path`
- `name`
- `kind`
- `expanded`
- `loaded`
- `loading`
- `error`
- `childPaths` for directories

Notes:
- files do not need `childPaths`
- ordering should remain directories first, files second, alphabetized by name
- the file viewer still owns this state locally; it should not move into global session store state

### 2. Update Model

Expand/collapse should update only the targeted node record.

New helper style:
- initialize root listing into `rootPaths + nodesByPath`
- merge a directory listing by replacing that directory's `childPaths` and updating only the affected child node records
- toggle a node by updating its `expanded` flag directly

The new model should eliminate:
- full recursive `mapTreeNodes()` traversal for each state change
- separate recursive `findNode()` scan after toggling

Instead, lookups should be constant-time by `path`.

### 3. Rendering Model

Keep a recursive visual tree, but render from indexed state.

Recommended structure:
- the dialog renders top-level rows from `rootPaths`
- each row reads its own node record from `nodesByPath`
- directories render their children by iterating over `childPaths`

Add memoization to row rendering so unrelated subtrees do not rerender when a sibling node toggles.

Two acceptable implementations:
- `memo(FileTreeNodeRow)` with stable props
- split row state reading into a smaller wrapper so prop changes stay narrow

The key requirement is that toggling one node should not force broad rerenders of already-loaded sibling branches.

### 4. Loading Behavior

First expand of an unloaded directory should still:
- set `expanded=true`
- immediately show loading state for that node
- request `api.getFiles(sessionId, dirPath, signal)`
- merge children into the indexed tree when the request resolves

Repeated expand of an already loaded directory should:
- reuse cached children
- avoid a second fetch
- avoid whole-tree state traversal
- feel close to instant

Collapse should:
- preserve cached children
- only set `expanded=false`

### 5. Request and Abort Behavior

Keep per-directory request cancellation.

The existing `treeRequestControllersRef` map is still the right basic mechanism.

Refinements:
- one active request per directory path
- collapsing a directory does not have to discard loaded children
- reopening a previously loaded directory should not restart a request unless the prior load failed and the user retries

### 6. Optional Perceived-Performance Improvement

After the structural fix, add a conservative warmup step for only the highest-probability next interactions.

Recommended scope:
- after loading the root directory, optionally prefetch direct children for a very small number of visible top-level directories
- keep this bounded to avoid recreating the old full-tree eager-loading problem

This is explicitly secondary. Structural state-update optimization is the primary fix.

### 7. Non-Goals

This change should not:
- redesign the file viewer layout
- change file read or diff APIs
- move file-tree state into `web/src/domains/session-ui/store.ts`
- reintroduce recursive full-repository listing
- add virtualization in this iteration unless measurements still show rendering as the dominant bottleneck after the indexed-state fix

### 8. Why This Approach

Compared with smaller patching:
- adding only `memo` helps rerender cost but leaves recursive state updates in place
- reducing only fetch latency helps first expand but not repeated expand/collapse

The indexed-state approach directly addresses the user's observed symptom pattern:
- first expand improves somewhat because local work drops
- repeated expand improves substantially because it becomes mostly a single-node toggle plus narrow rerendering

## Implementation Outline

1. Introduce a `TreeNodeRecord` shape and indexed tree helpers in `FileViewerDialog.tsx`
2. Replace nested-tree update helpers with path-indexed update helpers
3. Remove recursive target lookup after toggle and replace it with direct path lookup
4. Update row rendering to read child relationships from `childPaths`
5. Memoize row rendering so unaffected branches stay untouched
6. Optionally add a very small top-level prefetch pass if needed after measurement

## Testing Plan

Add or update frontend tests around `FileViewerDialog` and app-shell integration to verify:
- opening the dialog still requests only the root directory listing
- first expand of an unloaded directory triggers one directory request and shows local loading state
- repeated expand of a loaded directory does not refetch
- collapse and re-expand preserve cached children
- retry after a failed directory load still works
- unrelated loaded branches remain stable when toggling a sibling directory

If practical, add a focused test around the indexed update helpers to confirm that toggling one node does not mutate unrelated node records.

## Acceptance Criteria

The change is successful if:
- first expand remains lazy-loaded but feels more responsive
- repeated expand/collapse of an already loaded directory is noticeably faster
- directory toggles no longer require recursive full-tree updates
- cached child listings survive collapse/re-expand within the dialog session
- the current file browser behavior remains correct on desktop and mobile
- tests cover the new lazy-tree state behavior

## Risks and Mitigations

Risk: indexed state can become inconsistent with parent-child relationships.
Mitigation: centralize merging logic in a small set of helper functions and test them.

Risk: row memoization may be defeated by unstable props.
Mitigation: pass only primitive props or stable references needed by each row.

Risk: light prefetch could grow back into eager loading.
Mitigation: keep prefetch optional, shallow, and strictly bounded to a tiny number of top-level directories.
