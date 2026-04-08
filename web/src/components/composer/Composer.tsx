import { useMemo, useState } from "preact/hooks";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

import {
  useComposerStore,
  useComposerStoreApi,
  useSessionUiStore,
  useSessionsStore,
  useSessionsStoreApi,
} from "../../app/providers";
import { getDisplayableTodoSnapshot, TodoComposerPanel } from "./TodoComposerPanel";

function enterToSendEnabled() {
  return window.localStorage.getItem("codoxear.enterToSend") === "1";
}

export function Composer() {
  const { activeSessionId, items } = useSessionsStore();
  const { draft, sending } = useComposerStore();
  const { sessionId: sessionUiSessionId, diagnostics } = useSessionUiStore();
  const sessionsStoreApi = useSessionsStoreApi();
  const composerStoreApi = useComposerStoreApi();
  const [todoExpandedBySessionId, setTodoExpandedBySessionId] = useState<Record<string, boolean>>({});
  const activeSession = items.find((session) => session.session_id === activeSessionId) ?? null;
  const todoSnapshot = useMemo(() => {
    if (!activeSessionId || activeSession?.agent_backend !== "pi") {
      return null;
    }

    if (sessionUiSessionId !== activeSessionId) {
      return null;
    }

    if (!diagnostics || typeof diagnostics !== "object") {
      return null;
    }

    const snapshot = (diagnostics as { todo_snapshot?: unknown }).todo_snapshot;

    return getDisplayableTodoSnapshot(snapshot);
  }, [activeSession?.agent_backend, activeSessionId, diagnostics, sessionUiSessionId]);

  const visibleTodoExpanded = activeSessionId ? Boolean(todoExpandedBySessionId[activeSessionId]) : false;

  return (
    <div className="composerStack space-y-3">
      {todoSnapshot ? (
        <TodoComposerPanel
          snapshot={todoSnapshot}
          expanded={visibleTodoExpanded}
          onToggle={() => {
            const currentSessionId = sessionsStoreApi.getState().activeSessionId;

            if (!currentSessionId) {
              return;
            }

            setTodoExpandedBySessionId((value) => ({
              ...value,
              [currentSessionId]: !value[currentSessionId],
            }));
          }}
        />
      ) : null}
      <Card
        data-testid="composer-card"
        className="composerCard rounded-[1.5rem] border-border/70 bg-card/95 shadow-lg shadow-primary/5 backdrop-blur-sm"
      >
        <CardContent className="p-3 sm:p-4">
          <form
            className={cn("composer composerShell flex items-end gap-2", draft.includes("\n") && "multiline")}
            onSubmit={(event) => {
              event.preventDefault();
              if (activeSessionId) {
                composerStoreApi.submit(activeSessionId);
              }
            }}
          >
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="composerAttachButton h-12 w-12 rounded-2xl border-border/70 bg-background/80"
              aria-label="Attach file"
            >
              <span className="buttonGlyph">📎</span>
              <span className="visuallyHidden">Attach file</span>
            </Button>
            <div className="composerInputWrap flex-1">
              <Textarea
                value={draft}
                placeholder="Enter your instructions here"
                className="composerTextarea min-h-[3.5rem] rounded-2xl border-border/70 bg-background/80 px-4 py-3 text-base shadow-none focus-visible:ring-2"
                onInput={(event) => composerStoreApi.setDraft(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || event.isComposing) {
                    return;
                  }
                  if (event.shiftKey) {
                    return;
                  }
                  if (!enterToSendEnabled() && !event.ctrlKey && !event.metaKey) {
                    return;
                  }
                  if (!activeSessionId) {
                    return;
                  }
                  event.preventDefault();
                  composerStoreApi.submit(activeSessionId).catch(() => undefined);
                }}
                disabled={sending}
              />
            </div>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="composerQueueButton h-12 w-12 rounded-2xl"
              aria-label="Queued messages"
            >
              <span className="buttonGlyph">≡</span>
              <span className="visuallyHidden">Queued messages</span>
            </Button>
            <Button
              type="submit"
              className="sendButton h-12 rounded-2xl px-4"
              aria-label={sending ? "Sending" : "Send"}
              disabled={sending || !draft.trim()}
            >
              <span className="buttonGlyph">➤</span>
              <span className="visuallyHidden">{sending ? "Sending..." : "Send"}</span>
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
