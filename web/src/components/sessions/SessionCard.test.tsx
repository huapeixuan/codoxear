import { render } from "preact";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "../../app/providers";
import type { SessionSummary } from "../../lib/types";
import { SessionCard } from "./SessionCard";

vi.mock("../../lib/api", () => ({
  api: {
    getSessionRepo: vi.fn(),
  },
}));

let root: HTMLDivElement | null = null;

afterEach(() => {
  if (root) {
    render(null, root);
    root.remove();
    root = null;
  }
  vi.restoreAllMocks();
});

function mount(session: SessionSummary) {
  root = document.createElement("div");
  document.body.appendChild(root);
  render(
    <AppProviders>
      <SessionCard session={session} active={false} onSelect={() => {}} />
    </AppProviders>,
    root,
  );
  return root;
}

function baseSession(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_id: "sess-1",
    alias: "Inbox cleanup",
    agent_backend: "codex",
    busy: false,
    ...overrides,
  };
}

describe("SessionCard repo badges", () => {
  it("shows branch and PR badges when both are present", () => {
    const container = mount(
      baseSession({
        git_branch: "feature/login",
        pr_summary: { number: 42, state: "OPEN" },
      }),
    );
    const badges = container.querySelector('[data-testid="repo-badges"]');
    expect(badges).not.toBeNull();
    expect(badges?.textContent).toContain("feature/login");
    expect(badges?.textContent).toContain("#42");
  });

  it("shows only the branch badge when no PR summary is set", () => {
    const container = mount(baseSession({ git_branch: "main", pr_summary: null }));
    expect(container.querySelector('[data-testid="repo-badges"]')?.textContent).toContain("main");
    expect(container.querySelector('[data-testid="pr-badge"]')).toBeNull();
  });

  it("omits the badges row entirely when no git context is set", () => {
    const container = mount(baseSession());
    expect(container.querySelector('[data-testid="repo-badges"]')).toBeNull();
  });

  it("opens the PR URL in a new tab when the PR badge is clicked", async () => {
    const { api } = await import("../../lib/api");
    const mocked = api.getSessionRepo as unknown as ReturnType<typeof vi.fn>;
    mocked.mockResolvedValueOnce({
      cwd: "/tmp",
      git_branch: "feature/login",
      pr: {
        number: 42,
        title: "x",
        state: "OPEN",
        url: "https://github.com/example/repo/pull/42",
        is_draft: false,
        head_ref_name: "feature/login",
      },
      availability: "ok",
    });
    const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);

    const container = mount(
      baseSession({ git_branch: "feature/login", pr_summary: { number: 42, state: "OPEN" } }),
    );
    const prBadge = container.querySelector('[data-testid="pr-badge"]') as HTMLElement | null;
    expect(prBadge).not.toBeNull();
    prBadge!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await Promise.resolve();
    await Promise.resolve();
    expect(mocked).toHaveBeenCalledWith("sess-1");
    expect(openSpy).toHaveBeenCalledWith(
      "https://github.com/example/repo/pull/42",
      "_blank",
      "noopener,noreferrer",
    );
  });
});
