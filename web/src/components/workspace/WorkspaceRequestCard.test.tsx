import { render } from "preact";
import { afterEach, describe, expect, it, vi } from "vitest";

import { WorkspaceRequestCard } from "./WorkspaceRequestCard";

describe("WorkspaceRequestCard", () => {
  let root: HTMLDivElement | null = null;

  afterEach(() => {
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
    vi.clearAllMocks();
  });

  it("renders a confirm request with confirm and cancel actions", () => {
    root = document.createElement("div");
    document.body.appendChild(root);

    render(
      <WorkspaceRequestCard
        request={{ id: "req-1", method: "confirm", question: "Continue with deploy?", context: "Need explicit confirmation." } as any}
        sessionId="sess-1"
        draftValue=""
        freeformValue=""
        askUserBridgeAnswers={{}}
        submitting={false}
        errorMessage=""
        onDraftChange={vi.fn()}
        onFreeformChange={vi.fn()}
        onAskUserBridgeAnswerChange={vi.fn()}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
      root,
    );

    expect(root.textContent).toContain("Continue with deploy?");
    expect(root.textContent).toContain("Need explicit confirmation.");
    expect(root.textContent).toContain("Confirm");
    expect(root.textContent).toContain("Cancel");
  });
});
