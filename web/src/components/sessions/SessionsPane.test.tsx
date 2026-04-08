import { render } from "preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppProviders } from "../../app/providers";
import { SessionsPane } from "./SessionsPane";

function createStaticStore(state: any) {
  return {
    getState: () => state,
    subscribe: () => () => undefined,
    refresh: vi.fn(),
    select: vi.fn(),
  };
}

describe("SessionsPane", () => {
  let root: HTMLDivElement | null = null;

  afterEach(() => {
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
  });

  it("renders the sessions surface with active session cards and metadata badges", () => {
    const sessionsStore = createStaticStore({
      items: [
        {
          session_id: "sess-1",
          alias: "Inbox cleanup",
          first_user_message: "整理一下今天的会话",
          agent_backend: "pi",
          busy: true,
          owned: true,
          queue_len: 2,
        },
      ],
      activeSessionId: "sess-1",
      loading: false,
      newSessionDefaults: null,
    });

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any}>
        <SessionsPane />
      </AppProviders>,
      root,
    );

    expect(root.querySelector("[data-testid='sessions-surface']")).not.toBeNull();
    expect(root.querySelectorAll("[data-testid='session-card']")).toHaveLength(1);
    const activeCard = root.querySelector<HTMLButtonElement>("[data-testid='session-card'].ring-2");
    expect(activeCard).not.toBeNull();
    expect(activeCard?.getAttribute("aria-current")).toBe("true");
    expect(root.textContent).toContain("Inbox cleanup");
    expect(root.textContent).toContain("pi");
    expect(root.textContent).toContain("web");
  });

  it("uses the first user message as the primary title when no alias is present", () => {
    const sessionsStore = createStaticStore({
      items: [
        {
          session_id: "4a145abccb9a48889dc7f3e5bed735f2",
          first_user_message: "我准备用 preact + vite 重构web端，请帮我出个规划",
          cwd: "/Users/huapeixuan/Documents/Code/codoxear",
          agent_backend: "pi",
        },
      ],
      activeSessionId: null,
      loading: false,
      newSessionDefaults: null,
    });

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any}>
        <SessionsPane />
      </AppProviders>,
      root,
    );

    const title = root.querySelector(".sessionTitle")?.textContent || "";
    const preview = root.querySelector(".sessionPreview")?.textContent || "";
    expect(title).toContain("我准备用 preact + vite 重构web端，请帮我出个规划");
    expect(preview).toContain("/Users/huapeixuan/Documents/Code/codoxear");
    expect(title).not.toContain("4a145abccb9a48889dc7f3e5bed735f2");
  });
});
