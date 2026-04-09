import { useMemo, useState } from "preact/hooks";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

import { useSessionsStore, useSessionsStoreApi } from "../../app/providers";
import { api } from "../../lib/api";
import type { CwdGroupMeta, SessionSummary } from "../../lib/types";
import { normalizeLaunchBackend, providerChoiceToSettings } from "../../lib/launch";
import { EditSessionDialog } from "./EditSessionDialog";
import { SessionCard } from "./SessionCard";
import { SessionGroup } from "./SessionGroup";

interface SessionsPaneProps {
  onNewSession?: () => void;
}

const FALLBACK_GROUP_KEY = "__no_working_directory__";
const FALLBACK_GROUP_TITLE = "No working directory";
const FALLBACK_GROUP_SUBTITLE = "Sessions without a cwd";

type SessionWithStartTs = SessionSummary & { start_ts?: number };

interface GroupedSessions {
  key: string;
  cwd: string | null;
  title: string;
  subtitle: string;
  collapsed: boolean;
  freshestTs: number;
  firstIndex: number;
  sessions: SessionSummary[];
}

function shortSessionId(sessionId: string) {
  const match = sessionId.match(/^([0-9a-f]{8})[0-9a-f-]{20,}$/i);
  return match ? match[1] : sessionId.slice(0, 8);
}

function deleteSessionConfirmText(session: SessionSummary) {
  const name = session.alias || session.first_user_message || session.title || "";
  const sid = shortSessionId(session.session_id);
  const target = name ? ` \"${name}\" (${sid})` : ` ${sid}`;
  if (session.owned) {
    return `Delete this web-owned session${target}? This will stop it and remove it from Codoxear.`;
  }
  return `Delete this terminal-owned session${target}? This will also stop the corresponding terminal session.`;
}

function getSessionActivityTs(session: SessionSummary) {
  const candidate = session as SessionWithStartTs;
  return Number(candidate.updated_ts ?? candidate.start_ts ?? 0) || 0;
}

function getGroupTitle(cwd: string | null) {
  if (!cwd) {
    return FALLBACK_GROUP_TITLE;
  }
  const parts = cwd.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] || cwd;
}

function groupSessions(items: SessionSummary[], cwdGroups: Record<string, CwdGroupMeta>) {
  const groups = new Map<string, GroupedSessions>();

  items.forEach((session, index) => {
    const cwd = session.cwd?.trim() || null;
    const key = cwd || FALLBACK_GROUP_KEY;
    const meta = cwd ? cwdGroups[cwd] : undefined;
    const existing = groups.get(key);

    if (existing) {
      existing.sessions.push(session);
      existing.freshestTs = Math.max(existing.freshestTs, getSessionActivityTs(session));
      return;
    }

    groups.set(key, {
      key,
      cwd,
      title: meta?.label?.trim() || getGroupTitle(cwd),
      subtitle: cwd || FALLBACK_GROUP_SUBTITLE,
      collapsed: Boolean(meta?.collapsed),
      freshestTs: getSessionActivityTs(session),
      firstIndex: index,
      sessions: [session],
    });
  });

  return Array.from(groups.values()).sort((left, right) => {
    if (right.freshestTs !== left.freshestTs) {
      return right.freshestTs - left.freshestTs;
    }
    return left.firstIndex - right.firstIndex;
  });
}

export function SessionsPane({ onNewSession }: SessionsPaneProps) {
  const { items, activeSessionId, cwdGroups = {} } = useSessionsStore();
  const sessionsStoreApi = useSessionsStoreApi();
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [actionError, setActionError] = useState("");
  const [pendingGroupKey, setPendingGroupKey] = useState<string | null>(null);

  const editingSession = useMemo(
    () => items.find((session) => session.session_id === editingSessionId) ?? null,
    [editingSessionId, items],
  );
  const groupedSessions = useMemo(() => groupSessions(items, cwdGroups), [items, cwdGroups]);

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

  const duplicateSession = async (session: SessionSummary) => {
    const cwd = String(session.cwd || "").trim();
    if (!cwd) {
      setActionError("This session does not have a working directory to duplicate.");
      return;
    }

    setActionError("");
    const backend = normalizeLaunchBackend(session.agent_backend);
    const providerSettings = providerChoiceToSettings(String(session.provider_choice || ""), backend);

    try {
      const response = await api.createSession({
        cwd,
        backend,
        model: String(session.model || "").trim() || undefined,
        model_provider: providerSettings.model_provider,
        preferred_auth_method: providerSettings.preferred_auth_method,
        reasoning_effort: String(session.reasoning_effort || "").trim() || undefined,
        service_tier: String(session.service_tier || "").trim().toLowerCase() === "fast" ? "fast" : undefined,
        create_in_tmux: backend === "codex" && String(session.transport || "").trim().toLowerCase() === "tmux" ? true : undefined,
      });

      await sessionsStoreApi.refresh();
      let createdSession = sessionsStoreApi.getState().items.find((item) => item.broker_pid === response.broker_pid);
      if (!createdSession) {
        await sessionsStoreApi.refresh({ preferNewest: true });
        const state = sessionsStoreApi.getState();
        createdSession = state.items.find((item) => item.broker_pid === response.broker_pid)
          ?? state.items.find((item) => item.session_id === state.activeSessionId)
          ?? state.items[0];
      }
      if (createdSession) {
        sessionsStoreApi.select(createdSession.session_id);
      }
    } catch (error) {
      setActionError(error instanceof Error ? error.message : "Failed to duplicate session");
    }
  };

  const toggleGroup = async (group: GroupedSessions) => {
    if (!group.cwd || pendingGroupKey === group.key) {
      return;
    }
    try {
      setActionError("");
      setPendingGroupKey(group.key);
      await api.editCwdGroup({
        cwd: group.cwd,
        label: group.title,
        collapsed: !group.collapsed,
      });
      await sessionsStoreApi.refresh();
    } catch (error) {
      setActionError(error instanceof Error ? error.message : "Failed to update session group");
    } finally {
      setPendingGroupKey(null);
    }
  };

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
        {actionError ? <p className="px-1 pb-2 text-sm font-medium text-red-600">{actionError}</p> : null}
        <ScrollArea className="sessionsSurfaceBody">
          <div className="sessionsList">
            {groupedSessions.map((group) => (
              <SessionGroup
                key={group.key}
                title={group.title}
                subtitle={group.subtitle}
                collapsed={group.collapsed}
                onToggle={group.cwd ? () => { void toggleGroup(group); } : undefined}
              >
                {group.sessions.map((session) => (
                  <SessionCard
                    key={session.session_id}
                    session={session}
                    active={session.session_id === activeSessionId}
                    onSelect={() => sessionsStoreApi.select(session.session_id)}
                    onEdit={() => {
                      setActionError("");
                      setEditingSessionId(session.session_id);
                    }}
                    onDuplicate={() => { void duplicateSession(session); }}
                    onDelete={() => { void deleteSession(session); }}
                  />
                ))}
              </SessionGroup>
            ))}
          </div>
        </ScrollArea>
      </aside>

      <EditSessionDialog
        key={editingSession?.session_id || "session-edit-dialog"}
        open={editingSession != null}
        session={editingSession}
        sessions={items}
        onClose={() => setEditingSessionId(null)}
        onSaved={async () => {
          await sessionsStoreApi.refresh();
        }}
      />
    </>
  );
}
