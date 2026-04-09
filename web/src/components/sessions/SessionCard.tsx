import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { cn } from "@/lib/utils";

import type { SessionSummary } from "../../lib/types";

interface SessionCardProps {
  session: SessionSummary;
  active: boolean;
  onSelect: () => void;
  onEdit?: () => void;
  onDuplicate?: () => void;
  onDelete?: () => void;
}

function shortSessionId(sessionId: string) {
  const match = sessionId.match(/^([0-9a-f]{8})[0-9a-f-]{20,}$/i);
  return match ? match[1] : sessionId.slice(0, 8);
}

export function useDesktopSessionActions() {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  return Boolean(window.matchMedia("(hover: hover) and (pointer: fine) and (min-width: 881px)").matches);
}

export function SessionCard({ session, active, onSelect, onEdit, onDuplicate, onDelete }: SessionCardProps) {
  const title = session.alias || session.first_user_message || session.title || shortSessionId(session.session_id);
  const preview = (session.alias ? session.first_user_message : session.cwd) || session.cwd || session.title || session.session_id;
  const desktopActions = useDesktopSessionActions();
  const hasActions = Boolean(onEdit || onDuplicate || onDelete);
  const showActions = hasActions && (!desktopActions || active);

  return (
    <div
      data-testid="session-card"
      className="sessionCard"
      aria-current={active ? "true" : undefined}
    >
      <Card className={cn("sessionCardSurface h-full border-border/60 bg-card/90 shadow-sm", active && "ring-1 ring-primary/30 shadow-md") }>
        <CardContent className="p-2.5">
          <button
            type="button"
            className="sessionCardButton w-full text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
            aria-current={active ? "true" : undefined}
            onClick={onSelect}
          >
            <div className="sessionMetaLine">
              <span className={cn("stateDot", session.busy && "busy")} />
              <Badge variant="secondary" className="backendBadge">{session.agent_backend || "codex"}</Badge>
              {session.owned ? <Badge variant="outline" className="ownerBadge">web</Badge> : null}
              {session.queue_len ? <Badge className="queueBadge">{session.queue_len} queued</Badge> : null}
            </div>
            <div className="sessionTitle">{title}</div>
            <div className="sessionPreview">{preview}</div>
          </button>
          {showActions ? (
            <div className="sessionActionRow mt-2 flex items-center justify-end gap-2">
              {onEdit ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="sessionEditButton"
                  onClick={() => onEdit()}
                >
                  Edit
                </Button>
              ) : null}
              {onDuplicate ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="sessionDuplicateButton"
                  onClick={() => onDuplicate()}
                >
                  Duplicate
                </Button>
              ) : null}
              {onDelete ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="sessionDeleteButton"
                  onClick={() => onDelete()}
                >
                  Delete
                </Button>
              ) : null}
            </div>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
