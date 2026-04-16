import { memo } from "preact/compat";
import { useCallback, useEffect, useRef, useState } from "preact/hooks";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";

import { api } from "../../lib/api";
import type { GitFileVersionsResponse, SessionFileListEntry, SessionFileReadResponse } from "../../lib/types";
import { MonacoWorkspace } from "./MonacoWorkspace";
import { normalizeRememberedLine, preferredFileSelectionForSession, rememberFileSelection } from "./fileSelectionState";
import {
  createTreeStateFromEntries,
  mergeDirectoryEntries,
  setNodeError,
  setNodeExpanded,
  setNodeLoading,
  type FileTreeState,
  type TreeNodeRecord,
} from "./fileTreeState";

export interface FileViewerDialogProps {
  open: boolean;
  sessionId: string | null;
  initialPath?: string;
  initialLine?: number | null;
  initialMode?: FileViewMode | null;
  openRequestKey?: number;
  onClose: () => void;
}

export type FileViewMode = "diff" | "file" | "preview";

const EMPTY_TREE_STATE: FileTreeState = {
  rootPaths: [],
  nodesByPath: {},
};

function getChildNodes(node: TreeNodeRecord, nodesByPath: Record<string, TreeNodeRecord>) {
  if (node.kind !== "dir") {
    return [];
  }
  return node.childPaths
    .map((childPath) => nodesByPath[childPath])
    .filter((child): child is TreeNodeRecord => Boolean(child));
}

function normalizeFileListEntries(value: unknown): SessionFileListEntry[] | null {
  if (!Array.isArray(value)) {
    return null;
  }

  return value.flatMap((entry) => {
    if (!entry || typeof entry !== "object") {
      return [];
    }
    const record = entry as Record<string, unknown>;
    const name = typeof record.name === "string" ? record.name.trim() : "";
    const path = typeof record.path === "string" ? record.path.trim() : "";
    const kind = record.kind === "dir" || record.kind === "file" ? record.kind : null;
    if (!name || !path || !kind) {
      return [];
    }
    return [{ name, path, kind }];
  });
}

function normalizePath(value: string) {
  return value.trim();
}

function normalizeViewMode(value?: FileViewMode | null) {
  if (value === "file" || value === "preview") {
    return value;
  }
  return "diff";
}

function isMarkdownFile(path: string) {
  return /\.(md|markdown)$/i.test(path.trim());
}

function renderMarkdownPreview(value: string) {
  const normalized = value.replace(/\r\n?/g, "\n");
  const blocks = normalized.split(/\n\n+/).filter(Boolean);

  return (
    <article className="space-y-4 text-sm leading-7 text-foreground">
      {blocks.map((block, index) => {
        const heading = block.match(/^#\s+(.+)$/m);
        if (heading) {
          return <h1 key={index} className="text-xl font-semibold">{heading[1]}</h1>;
        }
        return <p key={index}>{block.replace(/^#\s+.+$/m, "").trim()}</p>;
      })}
    </article>
  );
}

function PlainTextWorkspace({ value }: { value: string }) {
  return (
    <ScrollArea className="h-[58vh] rounded-2xl border border-border/60 bg-slate-950 p-4 text-slate-100" data-testid="file-text-view">
      <pre data-testid="file-text-body" className="whitespace-pre-wrap break-words text-sm leading-6">{value}</pre>
    </ScrollArea>
  );
}

function PlainDiffWorkspace({ baseText, currentText }: { baseText: string; currentText: string }) {
  return (
    <div className="grid h-[58vh] grid-cols-2 gap-4" data-testid="file-diff-view">
      <ScrollArea className="rounded-2xl border border-border/60 bg-slate-950 p-4 text-slate-100">
        <p className="mb-3 text-xs uppercase tracking-[0.14em] text-slate-400">Base</p>
        <pre data-testid="file-diff-base" className="whitespace-pre-wrap break-words text-sm leading-6">{baseText}</pre>
      </ScrollArea>
      <ScrollArea className="rounded-2xl border border-border/60 bg-slate-950 p-4 text-slate-100">
        <p className="mb-3 text-xs uppercase tracking-[0.14em] text-slate-400">Current</p>
        <pre data-testid="file-diff-current" className="whitespace-pre-wrap break-words text-sm leading-6">{currentText}</pre>
      </ScrollArea>
    </div>
  );
}

const FileTreeNodeRow = memo(function FileTreeNodeRow({
  depth,
  node,
  nodesByPath,
  onRetry,
  onSelect,
  onToggle,
  selectedPath,
}: {
  depth: number;
  node: TreeNodeRecord;
  nodesByPath: Record<string, TreeNodeRecord>;
  onRetry: (path: string) => void;
  onSelect: (path: string) => void;
  onToggle: (path: string, expanded: boolean) => void;
  selectedPath: string;
}) {
  const selected = node.path === selectedPath;
  const childNodes = getChildNodes(node, nodesByPath);

  return (
    <div className="space-y-1">
      <div className="flex items-center gap-1" style={{ paddingLeft: `${depth * 12}px` }}>
        {node.kind === "dir" ? (
          <button
            type="button"
            aria-label={node.expanded ? `Collapse ${node.name}` : `Expand ${node.name}`}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition hover:bg-accent hover:text-accent-foreground"
            onClick={() => onToggle(node.path, !node.expanded)}
          >
            {node.expanded ? "-" : "+"}
          </button>
        ) : (
          <span className="inline-block h-8 w-8" aria-hidden="true" />
        )}
        <button
          type="button"
          className={`min-w-0 flex-1 rounded-xl px-3 py-2 text-left text-sm transition hover:bg-accent hover:text-accent-foreground ${selected ? "bg-accent text-accent-foreground" : "text-foreground"}`}
          onClick={() => {
            if (node.kind === "dir") {
              onToggle(node.path, !node.expanded);
              return;
            }
            onSelect(node.path);
          }}
        >
          {node.name}
        </button>
      </div>
      {node.kind === "dir" && node.expanded ? (
        <div className="space-y-1">
          {node.loading ? <p className="px-3 text-sm text-muted-foreground" style={{ paddingLeft: `${depth * 12 + 40}px` }}>Loading…</p> : null}
          {!node.loading && node.error ? (
            <div className="flex items-center gap-2 px-3" style={{ paddingLeft: `${depth * 12 + 40}px` }}>
              <p className="text-sm text-destructive">{node.error}</p>
              <Button type="button" size="sm" variant="outline" onClick={() => onRetry(node.path)}>Retry</Button>
            </div>
          ) : null}
          {!node.loading && !node.error && childNodes.length
            ? childNodes.map((child) => (
                <FileTreeNodeRow
                  key={child.path}
                  depth={depth + 1}
                  node={child}
                  nodesByPath={nodesByPath}
                  onRetry={onRetry}
                  onSelect={onSelect}
                  onToggle={onToggle}
                  selectedPath={selectedPath}
                />
              ))
            : null}
        </div>
      ) : null}
    </div>
  );
}, (prev, next) => {
  if (prev.depth !== next.depth || prev.node !== next.node || prev.selectedPath !== next.selectedPath) {
    return false;
  }
  if (prev.onRetry !== next.onRetry || prev.onSelect !== next.onSelect || prev.onToggle !== next.onToggle) {
    return false;
  }
  const prevChildren = getChildNodes(prev.node, prev.nodesByPath);
  const nextChildren = getChildNodes(next.node, next.nodesByPath);
  if (prevChildren.length !== nextChildren.length) {
    return false;
  }
  for (let index = 0; index < prevChildren.length; index += 1) {
    if (prevChildren[index] !== nextChildren[index]) {
      return false;
    }
  }
  return true;
});

export function FileViewerDialog({
  open,
  sessionId,
  initialPath = "",
  initialLine = null,
  initialMode = null,
  openRequestKey = 0,
  onClose,
}: FileViewerDialogProps) {
  const rememberedSelection = preferredFileSelectionForSession(sessionId);
  const rememberedPath = rememberedSelection?.path || "";
  const [treeState, setTreeState] = useState<FileTreeState>(EMPTY_TREE_STATE);
  const [treeError, setTreeError] = useState("");
  const [treeLoading, setTreeLoading] = useState(false);
  const [path, setPath] = useState("");
  const [line, setLine] = useState<number | null>(null);
  const [viewMode, setViewMode] = useState<FileViewMode>("diff");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [payload, setPayload] = useState<SessionFileReadResponse | null>(null);
  const [diffPayload, setDiffPayload] = useState<GitFileVersionsResponse | null>(null);
  const nodesByPathRef = useRef(treeState.nodesByPath);
  const listRequestIdRef = useRef(0);
  const openRequestIdRef = useRef(0);
  const fileOpenAbortRef = useRef<AbortController | null>(null);
  const treeRequestControllersRef = useRef(new Map<string, AbortController>());

  nodesByPathRef.current = treeState.nodesByPath;

  useEffect(() => {
    if (!open || !sessionId) {
      return;
    }

    listRequestIdRef.current += 1;
    const requestId = listRequestIdRef.current;
    const controller = new AbortController();
    for (const activeController of treeRequestControllersRef.current.values()) {
      activeController.abort();
    }
    treeRequestControllersRef.current.clear();
    setTreeState(EMPTY_TREE_STATE);
    setTreeError("");
    setTreeLoading(true);

    void (async () => {
      try {
        const response = await api.getFiles(sessionId, undefined, controller.signal);
        if (controller.signal.aborted || requestId !== listRequestIdRef.current) {
          return;
        }
        const entries = normalizeFileListEntries(response.entries);
        if (!entries) {
          setTreeState(EMPTY_TREE_STATE);
          setTreeError("Unable to list files");
          setTreeLoading(false);
          return;
        }
        setTreeState(createTreeStateFromEntries(entries));
        setTreeError("");
        setTreeLoading(false);
      } catch (nextError) {
        if (controller.signal.aborted || requestId !== listRequestIdRef.current) {
          return;
        }
        if (nextError instanceof Error && nextError.name === "AbortError") {
          return;
        }
        setTreeState(EMPTY_TREE_STATE);
        setTreeError(nextError instanceof Error ? nextError.message : "Unable to list files");
        setTreeLoading(false);
      }
    })();

    return () => {
      controller.abort();
      for (const activeController of treeRequestControllersRef.current.values()) {
        activeController.abort();
      }
      treeRequestControllersRef.current.clear();
    };
  }, [open, openRequestKey, sessionId]);

  useEffect(() => {
    if (!open) {
      return;
    }

    const preferredPath = normalizePath(initialPath || rememberedPath || "");
    if (!preferredPath) {
      setPath("");
      setLine(null);
      setPayload(null);
      setDiffPayload(null);
      setError("");
      setViewMode("diff");
      return;
    }

    const preferredLine = initialPath ? normalizeRememberedLine(initialLine) : normalizeRememberedLine(rememberedSelection?.line);
    setPath(preferredPath);
    setLine(preferredLine);
    setViewMode(initialPath ? normalizeViewMode(initialMode || "file") : "diff");
    setError("");
  }, [initialLine, initialMode, initialPath, open, openRequestKey, rememberedPath, rememberedSelection?.line]);

  useEffect(() => {
    if (!open || !sessionId) {
      return;
    }
    const normalized = normalizePath(path);
    if (!normalized) {
      return;
    }
    rememberFileSelection(sessionId, normalized, line);
  }, [line, open, path, sessionId]);

  useEffect(() => {
    const normalized = normalizePath(path);
    if (!open || !sessionId || !normalized) {
      return;
    }

    openRequestIdRef.current += 1;
    const requestId = openRequestIdRef.current;
    fileOpenAbortRef.current?.abort();
    const controller = new AbortController();
    fileOpenAbortRef.current = controller;

    setLoading(true);
    setError("");

    const loadMode = viewMode === "preview" ? "file" : viewMode;

    void (async () => {
      try {
        if (loadMode === "diff") {
          const response = await api.getGitFileVersions(sessionId, normalized, controller.signal);
          if (controller.signal.aborted || requestId !== openRequestIdRef.current) return;
          setDiffPayload(response);
          setPayload(null);
        } else {
          const response = await api.getFileRead(sessionId, normalized, controller.signal);
          if (controller.signal.aborted || requestId !== openRequestIdRef.current) return;
          setPayload(response);
          setDiffPayload(null);
        }
      } catch (nextError) {
        if (controller.signal.aborted || requestId !== openRequestIdRef.current) return;
        if (nextError instanceof Error && nextError.name === "AbortError") {
          return;
        }
        setPayload(null);
        setDiffPayload(null);
        setError(nextError instanceof Error ? nextError.message : "Unable to read file");
      } finally {
        if (!controller.signal.aborted && requestId === openRequestIdRef.current) {
          setLoading(false);
        }
      }
    })();

    return () => {
      controller.abort();
    };
  }, [open, openRequestKey, path, sessionId, viewMode]);

  const normalizedPath = normalizePath(path);
  const canPreview = isMarkdownFile(normalizedPath);
  const activeLine = normalizeRememberedLine(line);

  const handleSelect = useCallback((nextPath: string) => {
    setPath(nextPath);
    setLine(null);
  }, []);

  const loadDirectory = useCallback((dirPath: string) => {
    if (!sessionId) {
      return;
    }
    treeRequestControllersRef.current.get(dirPath)?.abort();
    const controller = new AbortController();
    treeRequestControllersRef.current.set(dirPath, controller);
    setTreeState((current) => setNodeLoading(current, dirPath, true));

    void api.getFiles(sessionId, dirPath, controller.signal).then((response) => {
      if (controller.signal.aborted) {
        return;
      }
      const entries = normalizeFileListEntries(response.entries);
      if (!entries) {
        setTreeState((current) => setNodeError(current, dirPath, "Unable to list files"));
        return;
      }
      setTreeState((current) => mergeDirectoryEntries(current, dirPath, entries));
    }).catch((nextError) => {
      if (controller.signal.aborted || (nextError instanceof Error && nextError.name === "AbortError")) {
        return;
      }
      setTreeState((current) => setNodeError(current, dirPath, nextError instanceof Error ? nextError.message : "Unable to list files"));
    }).finally(() => {
      treeRequestControllersRef.current.delete(dirPath);
    });
  }, [sessionId]);

  const toggleDirectory = useCallback((dirPath: string, expanded: boolean) => {
    const target = nodesByPathRef.current[dirPath];
    setTreeState((current) => setNodeExpanded(current, dirPath, expanded));
    if (!expanded) {
      return;
    }
    if (target?.loaded) {
      return;
    }
    loadDirectory(dirPath);
  }, [loadDirectory]);

  return (
    <Dialog open={open}>
      <DialogContent className="fileViewerDialog max-w-6xl p-0" titleId="file-viewer-title">
        <DialogHeader className="border-b border-border/60 px-5 py-4">
          <div className="flex items-start justify-between gap-3">
            <div>
              <DialogTitle id="file-viewer-title">File viewer</DialogTitle>
              <p className="text-sm text-muted-foreground">
                {normalizedPath ? normalizedPath : "Choose a file from the session."}
                {activeLine && normalizedPath ? ` (line ${activeLine})` : ""}
              </p>
            </div>
            <Button type="button" variant="ghost" size="sm" onClick={onClose}>Close</Button>
          </div>
        </DialogHeader>
        <div className="grid min-h-[65vh] grid-cols-[260px_minmax(0,1fr)] gap-0 max-md:grid-cols-1">
          <aside className="border-r border-border/60 bg-muted/20 p-4 max-md:border-b max-md:border-r-0">
            <label className="mb-3 block text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Path</label>
            <Input
              value={path}
              onInput={(event) => {
                setPath(event.currentTarget.value);
                setLine(null);
              }}
              placeholder="src/app.tsx"
            />
            <ScrollArea className="mt-4 h-[calc(65vh-8rem)] rounded-2xl border border-border/60 bg-background/80 p-2 max-md:h-48">
              <div className="space-y-1">
                {treeLoading ? (
                  <p className="px-2 py-3 text-sm text-muted-foreground">Loading files…</p>
                ) : treeError ? (
                  <p className="px-2 py-3 text-sm text-destructive">{treeError}</p>
                ) : treeState.rootPaths.length ? (
                  treeState.rootPaths.map((entryPath) => {
                    const entry = treeState.nodesByPath[entryPath];
                    return entry ? (
                      <FileTreeNodeRow
                        key={entry.path}
                        depth={0}
                        node={entry}
                        nodesByPath={treeState.nodesByPath}
                        onRetry={loadDirectory}
                        onSelect={handleSelect}
                        onToggle={toggleDirectory}
                        selectedPath={normalizedPath}
                      />
                    ) : null;
                  })
                ) : (
                  <p className="px-2 py-3 text-sm text-muted-foreground">No files available.</p>
                )}
              </div>
            </ScrollArea>
          </aside>
          <section className="min-w-0 p-4">
            <div className="mb-4 flex flex-wrap items-center gap-2">
              <Button type="button" variant={viewMode === "diff" ? "default" : "outline"} size="sm" onClick={() => setViewMode("diff")} disabled={!normalizedPath}>Diff</Button>
              <Button type="button" variant={viewMode === "file" ? "default" : "outline"} size="sm" onClick={() => setViewMode("file")} disabled={!normalizedPath}>File</Button>
              <Button type="button" variant={viewMode === "preview" ? "default" : "outline"} size="sm" onClick={() => setViewMode("preview")} disabled={!normalizedPath || !canPreview}>Preview</Button>
            </div>
            {loading ? <p className="text-sm text-muted-foreground">Loading file…</p> : null}
            {!loading && error ? <p className="text-sm text-destructive">{error}</p> : null}
            {!loading && !error && viewMode === "diff" && diffPayload ? (
              <MonacoWorkspace
                mode="diff"
                path={normalizedPath}
                line={activeLine}
                originalText={String(diffPayload.base_text || "")}
                modifiedText={String(diffPayload.current_text || "")}
                fallback={<PlainDiffWorkspace baseText={String(diffPayload.base_text || "")} currentText={String(diffPayload.current_text || "")} />}
              />
            ) : null}
            {!loading && !error && payload?.kind === "image" && payload.image_url ? (
              <div className="flex h-[58vh] items-start justify-center overflow-auto rounded-2xl border border-border/60 bg-background/70 p-4">
                <img src={payload.image_url} alt={normalizedPath || "Session file"} className="max-h-full rounded-xl object-contain" />
              </div>
            ) : null}
            {!loading && !error && viewMode === "preview" && canPreview && payload?.kind !== "image" ? (
              <ScrollArea className="filePreview h-[58vh] rounded-2xl border border-border/60 bg-background/70 p-4" data-testid="file-preview-view">
                {renderMarkdownPreview(payload?.text || "")}
              </ScrollArea>
            ) : null}
            {!loading && !error && viewMode === "file" && payload?.kind !== "image" ? (
              <MonacoWorkspace
                mode="file"
                path={normalizedPath}
                line={activeLine}
                modifiedText={payload?.text || ""}
                fallback={<PlainTextWorkspace value={payload?.text || ""} />}
              />
            ) : null}
          </section>
        </div>
      </DialogContent>
    </Dialog>
  );
}
