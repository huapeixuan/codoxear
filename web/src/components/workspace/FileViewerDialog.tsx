import { useEffect, useMemo, useRef, useState } from "preact/hooks";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";

import { api } from "../../lib/api";
import type { GitFileVersionsResponse, SessionFileReadResponse } from "../../lib/types";
import { MonacoWorkspace } from "./MonacoWorkspace";
import { normalizeRememberedLine, preferredFileSelectionForSession, rememberFileSelection } from "./fileSelectionState";

export interface FileViewerDialogProps {
  open: boolean;
  sessionId: string | null;
  files: string[];
  initialPath?: string;
  initialLine?: number | null;
  initialMode?: FileViewMode | null;
  openRequestKey?: number;
  onClose: () => void;
}

export type FileViewMode = "diff" | "file" | "preview";

function uniquePaths(paths: Array<string | null | undefined>) {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of paths) {
    if (typeof value !== "string") continue;
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    result.push(trimmed);
  }
  return result;
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

export function FileViewerDialog({
  open,
  sessionId,
  files,
  initialPath = "",
  initialLine = null,
  initialMode = null,
  openRequestKey = 0,
  onClose,
}: FileViewerDialogProps) {
  const rememberedSelection = preferredFileSelectionForSession(sessionId);
  const rememberedPath = rememberedSelection?.path || "";
  const [listedPaths, setListedPaths] = useState<string[]>([]);
  const [path, setPath] = useState("");
  const [line, setLine] = useState<number | null>(null);
  const [viewMode, setViewMode] = useState<FileViewMode>("diff");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [payload, setPayload] = useState<SessionFileReadResponse | null>(null);
  const [diffPayload, setDiffPayload] = useState<GitFileVersionsResponse | null>(null);
  const listRequestIdRef = useRef(0);
  const openRequestIdRef = useRef(0);
  const fileOpenAbortRef = useRef<AbortController | null>(null);

  const availablePaths = useMemo(
    () => uniquePaths([initialPath, rememberedPath, ...files, ...listedPaths]),
    [files, initialPath, listedPaths, rememberedPath],
  );

  useEffect(() => {
    if (!open || !sessionId) {
      return;
    }

    listRequestIdRef.current += 1;
    const requestId = listRequestIdRef.current;
    const controller = new AbortController();

    void api.getFiles(sessionId, controller.signal).then((response) => {
      if (requestId !== listRequestIdRef.current) {
        return;
      }
      setListedPaths(uniquePaths(response.files));
    }).catch((nextError) => {
      if (controller.signal.aborted || requestId !== listRequestIdRef.current) {
        return;
      }
      if (nextError instanceof Error && nextError.name === "AbortError") {
        return;
      }
      setListedPaths([]);
    });

    return () => {
      controller.abort();
    };
  }, [open, openRequestKey, sessionId]);

  useEffect(() => {
    if (!open) {
      return;
    }

    const preferredPath = normalizePath(initialPath || rememberedPath || availablePaths[0] || "");
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
  }, [availablePaths, initialLine, initialMode, initialPath, open, openRequestKey, rememberedPath, rememberedSelection?.line]);

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
                {availablePaths.length ? (
                  availablePaths.map((entry) => (
                    <button
                      key={entry}
                      type="button"
                      className={`w-full rounded-xl px-3 py-2 text-left text-sm transition hover:bg-accent hover:text-accent-foreground ${entry === normalizedPath ? "bg-accent text-accent-foreground" : "text-foreground"}`}
                      onClick={() => {
                        setPath(entry);
                        setLine(null);
                      }}
                    >
                      {entry}
                    </button>
                  ))
                ) : (
                  <p className="px-2 py-3 text-sm text-muted-foreground">No tracked files yet.</p>
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
