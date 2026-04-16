import type { SessionFileListEntry } from "../../lib/types";

export interface TreeNodeRecord extends SessionFileListEntry {
  expanded: boolean;
  loaded: boolean;
  loading: boolean;
  error: string;
  childPaths: string[];
}

export interface FileTreeState {
  rootPaths: string[];
  nodesByPath: Record<string, TreeNodeRecord>;
}

function sortEntries(entries: SessionFileListEntry[]) {
  return entries.slice().sort((left, right) => {
    if (left.kind !== right.kind) {
      return left.kind === "dir" ? -1 : 1;
    }
    return left.name.localeCompare(right.name);
  });
}

function toNodeRecord(entry: SessionFileListEntry): TreeNodeRecord {
  return {
    ...entry,
    expanded: false,
    loaded: false,
    loading: false,
    error: "",
    childPaths: [],
  };
}

export function createTreeStateFromEntries(entries: SessionFileListEntry[]): FileTreeState {
  const sorted = sortEntries(entries);
  const nodesByPath: Record<string, TreeNodeRecord> = {};
  for (const entry of sorted) {
    nodesByPath[entry.path] = toNodeRecord(entry);
  }
  return {
    rootPaths: sorted.map((entry) => entry.path),
    nodesByPath,
  };
}

export function setNodeExpanded(state: FileTreeState, path: string, expanded: boolean): FileTreeState {
  const target = state.nodesByPath[path];
  if (!target || target.expanded === expanded) {
    return state;
  }
  return {
    ...state,
    nodesByPath: {
      ...state.nodesByPath,
      [path]: { ...target, expanded },
    },
  };
}

export function setNodeLoading(state: FileTreeState, path: string, loading: boolean): FileTreeState {
  const target = state.nodesByPath[path];
  if (!target) {
    return state;
  }
  return {
    ...state,
    nodesByPath: {
      ...state.nodesByPath,
      [path]: { ...target, loading, error: loading ? "" : target.error },
    },
  };
}

export function setNodeError(state: FileTreeState, path: string, error: string): FileTreeState {
  const target = state.nodesByPath[path];
  if (!target) {
    return state;
  }
  return {
    ...state,
    nodesByPath: {
      ...state.nodesByPath,
      [path]: { ...target, loading: false, error },
    },
  };
}

export function mergeDirectoryEntries(
  state: FileTreeState,
  path: string,
  entries: SessionFileListEntry[],
): FileTreeState {
  const target = state.nodesByPath[path];
  if (!target) {
    return state;
  }

  const sorted = sortEntries(entries);
  const nextNodesByPath = { ...state.nodesByPath };
  for (const entry of sorted) {
    const existing = nextNodesByPath[entry.path];
    nextNodesByPath[entry.path] = existing
      ? { ...existing, name: entry.name, kind: entry.kind }
      : toNodeRecord(entry);
  }

  nextNodesByPath[path] = {
    ...target,
    expanded: true,
    loaded: true,
    loading: false,
    error: "",
    childPaths: sorted.map((entry) => entry.path),
  };

  return {
    ...state,
    nodesByPath: nextNodesByPath,
  };
}
