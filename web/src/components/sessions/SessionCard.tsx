import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";

import type { SessionSummary } from "../../lib/types";

interface SessionCardProps {
  session: SessionSummary;
  active: boolean;
  onSelect: () => void;
}

function shortSessionId(sessionId: string) {
  const match = sessionId.match(/^([0-9a-f]{8})[0-9a-f-]{20,}$/i);
  return match ? match[1] : sessionId.slice(0, 8);
}

export function SessionCard({ session, active, onSelect }: SessionCardProps) {
  const title = session.alias || session.first_user_message || session.title || shortSessionId(session.session_id);
  const preview = (session.alias ? session.first_user_message : session.cwd) || session.cwd || session.title || session.session_id;

  return (
    <button
      type="button"
      data-testid="session-card"
      className={cn(
        "sessionCard rounded-[1.35rem] text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50",
        active && "active ring-2 ring-primary",
      )}
      aria-current={active ? "true" : undefined}
      onClick={onSelect}
    >
      <Card className="h-full border-border/70 bg-card/95 shadow-sm backdrop-blur-sm">
        <CardContent className="p-4">
          <div className="sessionMetaLine">
            <span className={cn("stateDot", session.busy && "busy")} />
            <Badge variant="secondary" className="backendBadge">{session.agent_backend || "codex"}</Badge>
            {session.owned ? <Badge variant="outline" className="ownerBadge">web</Badge> : null}
            {session.queue_len ? <Badge className="queueBadge">{session.queue_len} queued</Badge> : null}
          </div>
          <div className="sessionTitle">{title}</div>
          <div className="sessionPreview">{preview}</div>
        </CardContent>
      </Card>
    </button>
  );
}
