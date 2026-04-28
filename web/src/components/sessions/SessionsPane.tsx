import { useMemo, useState } from "preact/hooks";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

import { useSessionsStore, useSessionsStoreApi } from "../../app/providers";
import { api } from "../../lib/api";
import { normalizeLaunchBackend, providerChoiceToSettings } from "../../lib/launch";
import type { CwdGroupMeta, SessionSummary } from "../../lib/types";
import { EditSessionDialog } from "./EditSessionDialog";
import { SessionCard } from "./SessionCard";
import { SessionGroup } from "./SessionGroup";

interface SessionsPaneProps {
  onNewSession?: () => void;
}

const FALLBACK_GROUP_KEY = "__no_working_directory__";
const FALLBACK_GROUP_TITLE = "No working directory";
const FALLBACK_GROUP_SUBTITLE = "Sessions without a cwd";

interface GroupedSessions {
  key: string;
  cwd: string | null;
  title: string;
  subtitle: string;
  collapsed: boolean;
  latestLiveStartTs: number | null;
  sessions: SessionSummary[];
}

function shortSessionId(sessionId: string) {
  const match = sessionId.match(/^([0-9a-f]{8})[0-9a-f-]{20,}$/i);
  return match ? match[1] : sessionId.slice(0, 8);
}

function historicalResumeSessionId(session: SessionSummary) {
  const explicit = String(session.resume_session_id || "").trim();
  if (explicit) {
    return explicit;
  }
  if (session.historical !== true) {
    return "";
  }
  const rawSessionId = String(session.session_id || "").trim();
  if (!rawSessionId.startsWith("history:")) {
    return "";
  }
  const parts = rawSessionId.split(":", 3);
  return parts.length === 3 ? String(parts[2] || "").trim() : "";
}

function sessionDisplayName(session: SessionSummary) {
  return String(session.alias || session.first_user_message || session.title || "").trim();
}

function sessionHasVisibleLabel(session: SessionSummary | null | undefined) {
  if (!session) {
    return false;
  }
  return Boolean(sessionDisplayName(session));
}

function deleteSessionConfirmText(session: SessionSummary) {
  const name = session.alias || session.first_user_message || session.title || "";
  const sid = shortSessionId(session.session_id);
  const target = name ? ` \"${name}\" (${sid})` : ` ${sid}`;
  if (session.historical) {
    return `Delete this historical session${target}? This will remove it from Codoxear history only.`;
  }
  if (session.owned) {
    return `Delete this web-owned session${target}? This will stop it and remove it from Codoxear.`;
  }
  return `Delete this terminal-owned session${target}? This will also stop the corresponding terminal session.`;
}

function getGroupTitle(cwd: string | null) {
  if (!cwd) {
    return FALLBACK_GROUP_TITLE;
  }
  const parts = cwd.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] || cwd;
}

function sessionRecentSubtitle(session: SessionSummary) {
  const cwd = String(session.cwd || "").trim();
  if (!cwd) {
    return "No working directory";
  }
  return cwd;
}

function groupSessions(items: SessionSummary[], cwdGroups: Record<string, CwdGroupMeta>) {
  const groups = new Map<string, GroupedSessions>();

  items.forEach((session) => {
    const cwd = session.cwd?.trim() || null;
    const key = cwd || FALLBACK_GROUP_KEY;
    const meta = cwd ? cwdGroups[cwd] : undefined;
    const existing = groups.get(key);

    if (existing) {
      existing.sessions.push(session);
      if (!session.historical && typeof session.start_ts === "number" && Number.isFinite(session.start_ts)) {
        existing.latestLiveStartTs = existing.latestLiveStartTs == null
          ? session.start_ts
          : Math.max(existing.latestLiveStartTs, session.start_ts);
      }
      return;
    }

    groups.set(key, {
      key,
      cwd,
      title: meta?.label?.trim() || getGroupTitle(cwd),
      subtitle: cwd || FALLBACK_GROUP_SUBTITLE,
      collapsed: Boolean(meta?.collapsed),
      latestLiveStartTs: !session.historical && typeof session.start_ts === "number" && Number.isFinite(session.start_ts)
        ? session.start_ts
        : null,
      sessions: [session],
    });
  });

  return Array.from(groups.values());
}

export function SessionsPane({ onNewSession }: SessionsPaneProps) {
  const {
    items,
    activeSessionId,
    viewMode,
    cwdGroups = {},
    remainingByGroup = {},
    omittedGroupCount = 0,
    remainingRecentCount = 0,
  } = useSessionsStore();
  const sessionsStoreApi = useSessionsStoreApi();
  const [editingSession, setEditingSession] = useState<SessionSummary | null>(null);
  const [actionError, setActionError] = useState("");
  const [pendingGroupKey, setPendingGroupKey] = useState<string | null>(null);
  const [groupErrors, setGroupErrors] = useState<Record<string, string>>({});

  const groupedSessions = useMemo(() => groupSessions(items, cwdGroups), [cwdGroups, items]);

  const deleteSession = async (session: SessionSummary) => {
    const confirmed = typeof window === "undefined" || typeof window.confirm !== "function"
      ? true
      : window.confirm(deleteSessionConfirmText(session));
    if (!confirmed) {
      return;
    }
    try {
      setActionError("");
      await api.deleteSession(session.session_id);
      await sessionsStoreApi.refresh();
    } catch (error) {
      setActionError(error instanceof Error ? error.message : "Failed to delete session");
    }
  };

  const selectCreatedSession = async (response: { session_id?: string; broker_pid?: number }) => {
    await sessionsStoreApi.refresh();
    const returnedSessionId = String(response.session_id || "").trim();
    let createdSession = returnedSessionId
      ? sessionsStoreApi.getState().items.find((item) => item.session_id === returnedSessionId)
      : undefined;
    if (!createdSession) {
      createdSession = sessionsStoreApi.getState().items.find((item) => item.broker_pid === response.broker_pid);
    }
    if (!createdSession) {
      await sessionsStoreApi.refresh({ preferNewest: true });
      const state = sessionsStoreApi.getState();
      createdSession = (returnedSessionId
        ? state.items.find((item) => item.session_id === returnedSessionId)
        : undefined)
        ?? state.items.find((item) => item.broker_pid === response.broker_pid)
        ?? state.items.find((item) => item.session_id === state.activeSessionId)
        ?? state.items[0];
    }
    if (createdSession) {
      sessionsStoreApi.select(createdSession.session_id);
    }
    return createdSession ?? null;
  };

  const resumeHistoricalSession = async (session: SessionSummary) => {
    setActionError("");

    try {
      let cwd = String(session.cwd || "").trim();
      let resumeSessionId = historicalResumeSessionId(session);
      let backend = normalizeLaunchBackend(session.agent_backend);

      // Historical rows already carry resume metadata; only fall back to details
      // if an older cached row is missing fields we now expect in the sidebar.
      if (!cwd || !resumeSessionId) {
        const details = await api.getSessionDetails(session.session_id);
        const source = details.session;
        cwd = cwd || String(source.cwd || "").trim();
        resumeSessionId = resumeSessionId || String(source.resume_session_id || "").trim();
        backend = normalizeLaunchBackend(source.agent_backend);
      }

      if (!cwd || !resumeSessionId) {
        setActionError("This historical session is missing resume metadata.");
        return;
      }

      const response = await api.createSession({
        cwd,
        backend,
        resume_session_id: resumeSessionId,
      });
      const createdSession = await selectCreatedSession(response);
      const createdSessionId = String(response.session_id || createdSession?.session_id || "").trim();
      const historicalLabel = sessionDisplayName(session);
      const needsRenamedTitle = historicalLabel
        && createdSessionId
        && (
          !createdSession
          || createdSession.session_id !== createdSessionId
          || !sessionHasVisibleLabel(createdSession)
        );
      if (needsRenamedTitle) {
        await api.renameSession(createdSessionId, historicalLabel);
        await sessionsStoreApi.refresh();
      }
      await sessionsStoreApi.refreshBootstrap();
    } catch (error) {
      setActionError(error instanceof Error ? error.message : "Failed to resume session");
    }
  };

  const duplicateSession = async (session: SessionSummary) => {
    const cwd = String(session.cwd || "").trim();
    if (!cwd) {
      setActionError("This session does not have a working directory to duplicate.");
      return;
    }

    setActionError("");

    try {
      const details = await api.getSessionDetails(session.session_id);
      const source = details.session;
      const backend = normalizeLaunchBackend(source.agent_backend);
      const providerSettings = providerChoiceToSettings(String(source.provider_choice || ""), backend);
      const response = await api.createSession({
        cwd,
        backend,
        model: String(source.model || "").trim() || undefined,
        model_provider: providerSettings.model_provider,
        preferred_auth_method: providerSettings.preferred_auth_method,
        reasoning_effort: String(source.reasoning_effort || "").trim() || undefined,
        service_tier: String(source.service_tier || "").trim().toLowerCase() === "fast" ? "fast" : undefined,
        create_in_tmux: backend === "codex" && String(source.transport || "").trim().toLowerCase() === "tmux" ? true : undefined,
      });
      await selectCreatedSession(response);
    } catch (error) {
      setActionError(error instanceof Error ? error.message : "Failed to duplicate session");
    }
  };

  async function saveGroupChange(group: GroupedSessions, payload: { label?: string; collapsed?: boolean; hidden?: boolean; hidden_after_live_start_ts?: number | null }) {
    if (!group.cwd) {
      return false;
    }

    setPendingGroupKey(group.key);
    setGroupErrors((current) => ({ ...current, [group.key]: "" }));

    try {
      await api.editCwdGroup({ cwd: group.cwd, ...payload });
      await sessionsStoreApi.refreshBootstrap();
      await sessionsStoreApi.refresh();
      return true;
    } catch (error) {
      const message = error instanceof Error ? error.message : "Failed to save group changes.";
      setGroupErrors((current) => ({ ...current, [group.key]: message }));
      return false;
    } finally {
      setPendingGroupKey((current) => (current === group.key ? null : current));
    }
  }

  return (
    <>
      <aside className="sessionsPane" data-testid="sessions-surface">
        <div className="sessionsSurfaceHeader">
          <div>
            <p className="sessionsEyebrow">Continue where you left off</p>
            <h2 className="sessionsSurfaceTitle">Sessions</h2>
          </div>
          <Button type="button" size="sm" className="sessionsNewButton" onClick={() => onNewSession?.()}>
            New session
          </Button>
        </div>
        <div className="flex gap-2 px-1 pb-3">
          <Button
            type="button"
            size="sm"
            variant={viewMode === "directories" ? "default" : "outline"}
            onClick={() => {
              void sessionsStoreApi.setViewMode("directories");
            }}
          >
            Directories
          </Button>
          <Button
            type="button"
            size="sm"
            variant={viewMode === "recent" ? "default" : "outline"}
            onClick={() => {
              void sessionsStoreApi.setViewMode("recent");
            }}
          >
            Recent
          </Button>
        </div>
        {actionError ? <p className="px-1 pb-2 text-sm font-medium text-red-600">{actionError}</p> : null}
        <ScrollArea className="sessionsSurfaceBody">
          <div className="sessionsList">
            {viewMode === "recent" ? items.map((session) => (
              <SessionCard
                key={session.session_id}
                session={session}
                active={session.session_id === activeSessionId}
                subtitle={sessionRecentSubtitle(session)}
                onSelect={() => {
                  if (session.historical && normalizeLaunchBackend(session.agent_backend) !== "pi") {
                    void resumeHistoricalSession(session);
                    return;
                  }
                  sessionsStoreApi.select(session.session_id);
                }}
                onDuplicate={session.historical ? undefined : () => { void duplicateSession(session); }}
                onDelete={() => { void deleteSession(session); }}
                onEdit={session.historical ? undefined : () => {
                  setActionError("");
                  void api.getSessionDetails(session.session_id)
                    .then((details) => setEditingSession(details.session))
                    .catch((error) => {
                      setActionError(error instanceof Error ? error.message : "Failed to load session details");
                    });
                }}
              />
            )) : groupedSessions.map((group) => {
              const visibleSessions = group.sessions;
              const hiddenSessionCount = Math.max(0, Number(remainingByGroup[group.key] || 0));
              const hasHiddenSessions = hiddenSessionCount > 0;

              return (
                <SessionGroup
                  key={group.key}
                  title={group.title}
                  subtitle={group.subtitle}
                  collapsed={group.collapsed}
                  canRename={Boolean(group.cwd)}
                  canHide={Boolean(group.cwd)}
                  isSaving={pendingGroupKey === group.key}
                  errorMessage={groupErrors[group.key]}
                  onRename={
                    group.cwd
                      ? async (label) => saveGroupChange(group, { label: label.trim() })
                      : undefined
                  }
                  onHide={
                    group.cwd
                      ? () => {
                          void saveGroupChange(group, {
                            hidden: true,
                            hidden_after_live_start_ts: group.latestLiveStartTs,
                          });
                        }
                      : undefined
                  }
                  onToggle={
                    group.cwd
                      ? () => {
                          void saveGroupChange(group, { collapsed: !group.collapsed });
                        }
                      : undefined
                  }
                >
                  {visibleSessions.map((session) => (
                    <SessionCard
                      key={session.session_id}
                      session={session}
                      active={session.session_id === activeSessionId}
                      onSelect={() => {
                        if (session.historical && normalizeLaunchBackend(session.agent_backend) !== "pi") {
                          void resumeHistoricalSession(session);
                          return;
                        }
                        sessionsStoreApi.select(session.session_id);
                      }}
                      onDuplicate={session.historical ? undefined : () => { void duplicateSession(session); }}
                      onDelete={() => { void deleteSession(session); }}
                      onEdit={session.historical ? undefined : () => {
                        setActionError("");
                        void api.getSessionDetails(session.session_id)
                          .then((details) => setEditingSession(details.session))
                          .catch((error) => {
                            setActionError(error instanceof Error ? error.message : "Failed to load session details");
                          });
                      }}
                    />
                  ))}
                  {hasHiddenSessions ? (
                    <button
                      type="button"
                      className="sessionGroupMoreButton"
                      aria-label={`Load ${hiddenSessionCount} more sessions in ${group.title}`}
                      onClick={() => {
                        void sessionsStoreApi.loadMoreGroup(group.key);
                      }}
                    >
                      ...
                    </button>
                  ) : null}
                </SessionGroup>
              );
            })}
            {viewMode === "recent" && remainingRecentCount > 0 ? (
              <button
                type="button"
                className="sessionGroupMoreButton"
                aria-label={`Load ${remainingRecentCount} more sessions`}
                onClick={() => {
                  void sessionsStoreApi.loadMoreRecent();
                }}
              >
                Load more sessions
              </button>
            ) : null}
            {viewMode === "directories" && omittedGroupCount > 0 ? (
              <button
                type="button"
                className="sessionGroupMoreButton"
                aria-label={`Load ${omittedGroupCount} more directories`}
                onClick={() => {
                  void sessionsStoreApi.loadMoreGroups();
                }}
              >
                Load {omittedGroupCount} more directories
              </button>
            ) : null}
          </div>
        </ScrollArea>
      </aside>

      <EditSessionDialog
        key={editingSession?.session_id || "session-edit-dialog"}
        open={editingSession != null}
        session={editingSession}
        sessions={items}
        onClose={() => setEditingSession(null)}
        onSaved={async () => {
          await sessionsStoreApi.refresh();
        }}
      />
    </>
  );
}
