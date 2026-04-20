import { useState } from "preact/hooks";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";

import type { PrState, PrSummary } from "../../lib/types";

interface RepoBadgesProps {
  sessionId: string;
  branch?: string | null;
  prSummary?: PrSummary | null;
}

function BranchIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.4">
      <circle cx="4" cy="3.5" r="1.4" />
      <circle cx="4" cy="12.5" r="1.4" />
      <circle cx="12" cy="5.5" r="1.4" />
      <path d="M4 4.9v6.2" />
      <path d="M4 9.5c0-2.2 1.8-4 4-4h2.6" />
    </svg>
  );
}

function prBadgeClass(state: PrState | string): string {
  switch (state) {
    case "OPEN":
      return "prBadge prBadgeOpen";
    case "DRAFT":
      return "prBadge prBadgeDraft";
    case "MERGED":
      return "prBadge prBadgeMerged";
    case "CLOSED":
      return "prBadge prBadgeClosed";
    default:
      return "prBadge";
  }
}

export function BranchBadge({ branch }: { branch: string }) {
  return (
    <Badge
      variant="outline"
      className="branchBadge inline-flex items-center gap-1"
      title={`Branch: ${branch}`}
    >
      <span className="branchBadgeIcon h-3 w-3" aria-hidden="true">
        <BranchIcon />
      </span>
      <span className="branchBadgeLabel truncate max-w-[10rem]">{branch}</span>
    </Badge>
  );
}

export function PrBadge({
  sessionId,
  summary,
}: {
  sessionId: string;
  summary: PrSummary;
}) {
  const [pending, setPending] = useState(false);

  const handleClick = async (event: { preventDefault: () => void; stopPropagation: () => void }) => {
    event.preventDefault();
    event.stopPropagation();
    if (pending) return;
    setPending(true);
    try {
      const detail = await api.getSessionRepo(sessionId);
      if (detail.pr?.url) {
        window.open(detail.pr.url, "_blank", "noopener,noreferrer");
      }
    } catch {
      // swallow – PR URL is best-effort; badge stays visible.
    } finally {
      setPending(false);
    }
  };

  const label = `PR #${summary.number} (${summary.state.toLowerCase()})`;
  return (
    <button
      type="button"
      className={cn(prBadgeClass(summary.state), "prBadgeButton inline-flex items-center gap-1")}
      aria-label={label}
      title={label}
      onClick={handleClick}
      data-testid="pr-badge"
    >
      <Badge variant="outline" className="prBadgeInner">
        #{summary.number}
      </Badge>
    </button>
  );
}

export function RepoBadges({ sessionId, branch, prSummary }: RepoBadgesProps) {
  const hasBranch = Boolean(branch && branch.length);
  const hasPr = Boolean(prSummary && prSummary.number > 0);
  if (!hasBranch && !hasPr) return null;
  return (
    <div className="repoBadges pointer-events-auto flex items-center gap-1" data-testid="repo-badges">
      {hasBranch && branch ? <BranchBadge branch={branch} /> : null}
      {hasPr && prSummary ? <PrBadge sessionId={sessionId} summary={prSummary} /> : null}
    </div>
  );
}
