import { useMemo, useState } from "preact/hooks";

import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

import { useSessionsStore, useSessionsStoreApi } from "../../app/providers";
import { api } from "../../lib/api";
import { normalizeLaunchBackend, providerChoiceToSettings } from "../../lib/launch";
import type { SessionSummary } from "../../lib/types";
import { EditSessionDialog } from "./EditSessionDialog";
import { SessionCard } from "./SessionCard";

interface SessionsPaneProps {
  onNewSession?: () => void;
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

export function SessionsPane({ onNewSession }: SessionsPaneProps) {
  const { items, activeSessionId } = useSessionsStore();
  const sessionsStoreApi = useSessionsStoreApi();
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [actionError, setActionError] = useState("");

  const editingSession = useMemo(
    () => items.find((session) => session.session_id === editingSessionId) ?? null,
    [editingSessionId, items],
  );

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
            {items.map((session) => (
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
