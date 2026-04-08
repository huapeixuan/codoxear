import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

import { useSessionsStore, useSessionsStoreApi } from "../../app/providers";
import { SessionCard } from "./SessionCard";

interface SessionsPaneProps {
  onNewSession?: () => void;
}

export function SessionsPane({ onNewSession }: SessionsPaneProps) {
  const { items, activeSessionId } = useSessionsStore();
  const sessionsStoreApi = useSessionsStoreApi();

  return (
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
      <ScrollArea className="sessionsSurfaceBody">
        <div className="sessionsList">
          {items.map((session) => (
            <SessionCard
              key={session.session_id}
              session={session}
              active={session.session_id === activeSessionId}
              onSelect={() => sessionsStoreApi.select(session.session_id)}
            />
          ))}
        </div>
      </ScrollArea>
    </aside>
  );
}
