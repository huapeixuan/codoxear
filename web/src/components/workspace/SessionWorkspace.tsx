import type { ComponentChildren } from "preact";
import { useMemo, useRef, useState } from "preact/hooks";

import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

import { useLiveSessionStore, useLiveSessionStoreApi, useSessionUiStore, useSessionUiStoreApi } from "../../app/providers";
import { api } from "../../lib/api";
import type { SessionUiRequest, TodoSnapshot } from "../../lib/types";
import { deriveWorkspaceViewData } from "./sessionWorkspaceViewModel";
import { WorkspaceRequestCard } from "./WorkspaceRequestCard";

type DraftValue = string | string[];
type AskUserBridgeAnswers = Record<string, string | string[]>;

function formatDiagnosticLabel(key: string): string {
  switch (key) {
    case "log_path":
      return "Log";
    case "session_file_path":
      return "Session file";
    case "updated_ts":
      return "Updated";
    case "cwd":
      return "Working directory";
    case "queue_len":
      return "Queue";
    default:
      return key.replace(/_/g, " ").replace(/\b\w/g, (char) => char.toUpperCase());
  }
}

function formatDiagnosticValue(key: string, value: unknown): string {
  if (key === "updated_ts" && typeof value === "number" && Number.isFinite(value) && value > 1_000_000_000) {
    return new Date(value * 1000).toLocaleString();
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return JSON.stringify(value);
}

function renderTodoSnapshotSection(snapshot: TodoSnapshot) {
  return (
    <div className="space-y-3 rounded-2xl border border-border/60 bg-card/60 p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-sm font-semibold text-foreground">Todo list</p>
          {snapshot.progress_text ? <p className="text-sm text-muted-foreground">{snapshot.progress_text}</p> : null}
        </div>
        <Badge variant="outline">{snapshot.available ? `${snapshot.items.length}` : "0"}</Badge>
      </div>
      {!snapshot.available ? (
        <p className="text-sm text-muted-foreground">{snapshot.error ? "Todo list unavailable" : "No todo list yet"}</p>
      ) : (
        <div className="space-y-2">
          {snapshot.items.map((item, index) => (
            <article key={`${item.title || "todo"}-${index}`} className="rounded-xl border border-border/60 bg-background/70 px-3 py-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <strong className="text-sm text-foreground">{item.title || "Untitled todo"}</strong>
                <Badge variant="secondary">{item.status || "unknown"}</Badge>
              </div>
              {item.description ? <p className="mt-2 text-sm text-muted-foreground">{item.description}</p> : null}
            </article>
          ))}
        </div>
      )}
    </div>
  );
}

function WorkspaceSection({
  title,
  badge,
  children,
}: {
  title: string;
  badge?: string;
  children: ComponentChildren;
}) {
  return (
    <section className="workspaceSurface space-y-3 rounded-[1.2rem] border border-border/70 bg-background/75 p-4 shadow-sm">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-foreground">{title}</h3>
        {badge ? <Badge variant="outline">{badge}</Badge> : null}
      </div>
      {children}
    </section>
  );
}

interface SessionWorkspaceProps {
  mode?: "default" | "details";
}

export function SessionWorkspace({ mode = "default" }: SessionWorkspaceProps) {
  const sessionUiState = useSessionUiStore() as {
    sessionId: string | null;
    diagnostics: Record<string, unknown> | null;
    queue: Record<string, unknown> | null;
    loading: boolean;
    requests?: SessionUiRequest[];
    files?: string[];
  };
  const { sessionId, diagnostics, queue, loading } = sessionUiState;
  const liveSessionState = useLiveSessionStore();
  const liveSessionStoreApi = useLiveSessionStoreApi();
  const liveRequests = sessionId ? liveSessionState.requestsBySessionId[sessionId] ?? [] : [];
  const requests = liveRequests.length ? liveRequests : Array.isArray(sessionUiState.requests) ? sessionUiState.requests : [];
  const files = Array.isArray(sessionUiState.files) ? sessionUiState.files : [];
  const sessionUiStoreApi = useSessionUiStoreApi();
  const [drafts, setDrafts] = useState<Record<string, DraftValue>>({});
  const [freeformDrafts, setFreeformDrafts] = useState<Record<string, string>>({});
  const [askUserBridgeDrafts, setAskUserBridgeDrafts] = useState<Record<string, AskUserBridgeAnswers>>({});
  const [requestSubmittingById, setRequestSubmittingById] = useState<Record<string, boolean>>({});
  const [requestErrorById, setRequestErrorById] = useState<Record<string, string>>({});
  const requestSubmittingIdsRef = useRef(new Set<string>());
  const {
    diagnosticsEntries,
    todoSnapshot,
    detailEntries,
    priorityDetailEntries,
    genericDetailEntries,
    queueItems,
  } = useMemo(() => deriveWorkspaceViewData({ diagnostics, queue }), [diagnostics, queue]);
  const showDetails = mode === "details";
  const hasWorkspaceData = diagnosticsEntries.length > 0 || queueItems.length > 0;
  const showTabs = showDetails || hasWorkspaceData || requests.length > 0;
  const defaultTab = useMemo(() => (
    showDetails
      ? "overview"
      : requests.length > 0
        ? "requests"
        : diagnosticsEntries.length > 0
          ? "diagnostics"
          : queueItems.length > 0
            ? "queue"
            : "requests"
  ), [diagnosticsEntries.length, queueItems.length, requests.length, showDetails]);

  const submitRequestResponse = async (requestId: string, payload: Record<string, unknown>) => {
    if (!sessionId || requestSubmittingIdsRef.current.has(requestId)) {
      return;
    }

    requestSubmittingIdsRef.current.add(requestId);
    setRequestSubmittingById((current) => ({ ...current, [requestId]: true }));
    setRequestErrorById((current) => ({ ...current, [requestId]: "" }));

    try {
      await api.submitUiResponse(sessionId, payload);
      await Promise.all([
        liveSessionStoreApi.loadInitial(sessionId),
        sessionUiStoreApi.refresh(sessionId, { agentBackend: "pi" }),
      ]);
    } catch (error) {
      const message = error instanceof Error ? error.message : "Failed to submit response";
      setRequestErrorById((current) => ({ ...current, [requestId]: message }));
    } finally {
      requestSubmittingIdsRef.current.delete(requestId);
      setRequestSubmittingById((current) => ({ ...current, [requestId]: false }));
    }
  };

  return (
    <aside className="workspacePane">
      <Card
        data-testid="workspace-card"
        className="workspaceCard flex h-full min-h-0 flex-col rounded-[1.5rem] border-border/70 bg-card/95 shadow-lg shadow-primary/5"
      >
        <CardHeader className="space-y-4 pb-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="space-y-1">
              <CardTitle className="text-base">Workspace</CardTitle>
              <p className="text-sm text-muted-foreground">
                {requests.length ? `${requests.length} pending UI request${requests.length === 1 ? "" : "s"}` : "No pending UI requests"}
              </p>
            </div>
            <Badge variant={loading ? "default" : hasWorkspaceData ? "secondary" : "outline"}>
              {loading ? "Refreshing" : hasWorkspaceData ? "Live context" : "Quiet"}
            </Badge>
          </div>
          {showTabs ? (
            <Tabs defaultValue={defaultTab} className="min-h-0 flex-1">
              <TabsList className="workspaceTabsList flex h-auto flex-wrap items-center gap-2 rounded-2xl bg-muted/60 p-1">
                {showDetails ? <TabsTrigger value="overview">Overview</TabsTrigger> : null}
                <TabsTrigger value="requests">UI Requests</TabsTrigger>
                <TabsTrigger value="diagnostics">Diagnostics</TabsTrigger>
                <TabsTrigger value="queue">Queue</TabsTrigger>
                {files.length ? <TabsTrigger value="files">Files</TabsTrigger> : null}
              </TabsList>
              <Separator className="bg-border/70" />
              <CardContent className="flex min-h-0 flex-1 flex-col p-0 pt-4">
                {showDetails ? (
                  <TabsContent value="overview" className="min-h-0">
                    <ScrollArea className="workspaceScroll h-full pr-1">
                      <div className="workspacePanelGrid grid gap-4 lg:grid-cols-2">
                        <WorkspaceSection title="Diagnostics" badge={diagnosticsEntries.length ? `${diagnosticsEntries.length}` : undefined}>
                          {detailEntries.length || todoSnapshot.available || todoSnapshot.error ? (
                            <div className="space-y-4">
                              {priorityDetailEntries.length ? (
                                <dl className="space-y-3">
                                  {priorityDetailEntries.map(([key, value]) => (
                                    <div key={key} className="grid gap-1 sm:grid-cols-[minmax(7rem,auto)_1fr] sm:gap-3">
                                      <dt className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">{formatDiagnosticLabel(key)}</dt>
                                      <dd className="m-0 break-all font-mono text-sm text-foreground">{formatDiagnosticValue(key, value)}</dd>
                                    </div>
                                  ))}
                                </dl>
                              ) : null}
                              {todoSnapshot.available || todoSnapshot.error ? renderTodoSnapshotSection(todoSnapshot) : null}
                              {genericDetailEntries.length ? (
                                <dl className="space-y-3">
                                  {genericDetailEntries.map(([key, value]) => (
                                    <div key={key} className="grid gap-1 sm:grid-cols-[minmax(7rem,auto)_1fr] sm:gap-3">
                                      <dt className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">{formatDiagnosticLabel(key)}</dt>
                                      <dd className="m-0 text-sm text-foreground">{formatDiagnosticValue(key, value)}</dd>
                                    </div>
                                  ))}
                                </dl>
                              ) : null}
                            </div>
                          ) : (
                            <p className="text-sm text-muted-foreground">No diagnostics available.</p>
                          )}
                        </WorkspaceSection>
                        <WorkspaceSection title="Queue" badge={queueItems.length ? `${queueItems.length}` : undefined}>
                          {queueItems.length ? (
                            <ul className="workspaceCollection space-y-2 text-sm text-foreground">
                              {queueItems.map((item, index) => (
                                <li key={`${item}-${index}`} className="rounded-xl border border-border/60 bg-card/60 px-3 py-2">
                                  {item}
                                </li>
                              ))}
                            </ul>
                          ) : (
                            <p className="text-sm text-muted-foreground">No queued items.</p>
                          )}
                        </WorkspaceSection>
                        {files.length ? (
                          <WorkspaceSection title="Files" badge={`${files.length}`}>
                            <ul className="workspaceCollection space-y-2 text-sm text-foreground">
                              {files.map((file) => (
                                <li key={file} className="rounded-xl border border-border/60 bg-card/60 px-3 py-2 font-mono text-xs sm:text-sm">
                                  {file}
                                </li>
                              ))}
                            </ul>
                          </WorkspaceSection>
                        ) : null}
                        <WorkspaceSection title="UI Requests" badge={requests.length ? `${requests.length}` : undefined}>
                          <p className="text-sm text-muted-foreground">
                            {requests.length ? "Review and respond in the dedicated tab." : "No pending requests."}
                          </p>
                        </WorkspaceSection>
                      </div>
                    </ScrollArea>
                  </TabsContent>
                ) : null}
                <TabsContent value="requests" className="min-h-0">
                  <ScrollArea className="workspaceScroll h-full pr-1">
                    <div className="space-y-4">
                      {requests.length ? (
                        requests.map((request, index) => {
                          const requestId = String(request.id ?? index);

                          return (
                            <WorkspaceRequestCard
                              key={requestId}
                              request={request}
                              sessionId={sessionId}
                              draftValue={drafts[requestId]}
                              freeformValue={freeformDrafts[requestId] ?? ""}
                              askUserBridgeAnswers={askUserBridgeDrafts[requestId] ?? {}}
                              submitting={Boolean(requestSubmittingById[requestId])}
                              errorMessage={requestErrorById[requestId] ?? ""}
                              onDraftChange={(value) => {
                                setDrafts((current) => ({ ...current, [requestId]: value }));
                              }}
                              onFreeformChange={(value) => {
                                setFreeformDrafts((current) => ({ ...current, [requestId]: value }));
                              }}
                              onAskUserBridgeAnswerChange={(question, value) => {
                                setAskUserBridgeDrafts((current) => ({
                                  ...current,
                                  [requestId]: {
                                    ...(current[requestId] ?? {}),
                                    [question]: value,
                                  },
                                }));
                              }}
                              onConfirm={(payload) => {
                                void submitRequestResponse(requestId, payload);
                              }}
                              onCancel={(payload) => {
                                void submitRequestResponse(requestId, payload);
                              }}
                            />
                          );
                        })
                      ) : (
                        <WorkspaceSection title="UI Requests">
                          <p className="text-sm text-muted-foreground">No pending requests.</p>
                        </WorkspaceSection>
                      )}
                    </div>
                  </ScrollArea>
                </TabsContent>
                <TabsContent value="diagnostics" className="min-h-0">
                  <ScrollArea className="workspaceScroll h-full pr-1">
                        <WorkspaceSection title="Diagnostics" badge={diagnosticsEntries.length ? `${diagnosticsEntries.length}` : undefined}>
                          {detailEntries.length || todoSnapshot.available || todoSnapshot.error ? (
                            <div className="space-y-4">
                              {priorityDetailEntries.length ? (
                                <dl className="space-y-3">
                                  {priorityDetailEntries.map(([key, value]) => (
                                    <div key={key} className="grid gap-1 sm:grid-cols-[minmax(7rem,auto)_1fr] sm:gap-3">
                                      <dt className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">{formatDiagnosticLabel(key)}</dt>
                                      <dd className="m-0 break-all font-mono text-sm text-foreground">{formatDiagnosticValue(key, value)}</dd>
                                    </div>
                                  ))}
                                </dl>
                              ) : null}
                              {todoSnapshot.available || todoSnapshot.error ? renderTodoSnapshotSection(todoSnapshot) : null}
                              {genericDetailEntries.length ? (
                                <dl className="space-y-3">
                                  {genericDetailEntries.map(([key, value]) => (
                                    <div key={key} className="grid gap-1 sm:grid-cols-[minmax(7rem,auto)_1fr] sm:gap-3">
                                      <dt className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">{formatDiagnosticLabel(key)}</dt>
                                      <dd className="m-0 text-sm text-foreground">{formatDiagnosticValue(key, value)}</dd>
                                    </div>
                                  ))}
                                </dl>
                              ) : null}
                            </div>
                          ) : (
                            <p className="text-sm text-muted-foreground">No diagnostics available.</p>
                          )}
                        </WorkspaceSection>
                  </ScrollArea>
                </TabsContent>
                <TabsContent value="queue" className="min-h-0">
                  <ScrollArea className="workspaceScroll h-full pr-1">
                    <WorkspaceSection title="Queue" badge={queueItems.length ? `${queueItems.length}` : undefined}>
                      {queueItems.length ? (
                        <ul className="workspaceCollection space-y-2 text-sm text-foreground">
                          {queueItems.map((item, index) => (
                            <li key={`${item}-${index}`} className="rounded-xl border border-border/60 bg-card/60 px-3 py-2">
                              {item}
                            </li>
                          ))}
                        </ul>
                      ) : (
                        <p className="text-sm text-muted-foreground">No queued items.</p>
                      )}
                    </WorkspaceSection>
                  </ScrollArea>
                </TabsContent>
                {files.length ? (
                  <TabsContent value="files" className="min-h-0">
                    <ScrollArea className="workspaceScroll h-full pr-1">
                      <WorkspaceSection title="Files" badge={`${files.length}`}>
                        <ul className="workspaceCollection space-y-2 text-sm text-foreground">
                          {files.map((file) => (
                            <li key={file} className="rounded-xl border border-border/60 bg-card/60 px-3 py-2 font-mono text-xs sm:text-sm">
                              {file}
                            </li>
                          ))}
                        </ul>
                      </WorkspaceSection>
                    </ScrollArea>
                  </TabsContent>
                ) : null}
              </CardContent>
            </Tabs>
          ) : (
            <>
              <Separator className="bg-border/70" />
              <CardContent className="pt-4">
                <WorkspaceSection title="UI Requests">
                  <p className="text-sm text-muted-foreground">No pending requests.</p>
                </WorkspaceSection>
              </CardContent>
            </>
          )}
        </CardHeader>
      </Card>
    </aside>
  );
}
