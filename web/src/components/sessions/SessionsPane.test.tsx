import { render } from "preact";
import { act } from "preact/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppProviders } from "../../app/providers";
import { SessionsPane } from "./SessionsPane";

vi.mock("../../lib/api", () => ({
  api: {
    createSession: vi.fn().mockResolvedValue({ ok: true, broker_pid: 42 }),
    editSession: vi.fn().mockResolvedValue({ ok: true, alias: "Updated session" }),
    deleteSession: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

function createStaticStore(state: any) {
  let currentState = state;
  const listeners = new Set<() => void>();
  return {
    getState: () => currentState,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    refresh: vi.fn(async () => undefined),
    select: vi.fn(),
    setState(next: any) {
      currentState = next;
      listeners.forEach((listener) => listener());
    },
  };
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
}

describe("SessionsPane", () => {
  let root: HTMLDivElement | null = null;

  afterEach(() => {
    vi.unstubAllGlobals();
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
    const activeCard = root.querySelector<HTMLButtonElement>("[data-testid='session-card'][aria-current='true']");
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

  it("deletes a session after confirmation and refreshes the list", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const confirm = vi.fn().mockReturnValue(true);
    vi.stubGlobal("confirm", confirm);
    const sessionsStore = createStaticStore({
      items: [
        {
          session_id: "sess-1",
          alias: "Inbox cleanup",
          first_user_message: "整理一下今天的会话",
          agent_backend: "pi",
        },
      ],
      activeSessionId: "sess-1",
      loading: false,
      newSessionDefaults: null,
    });
    sessionsStore.refresh = refresh;

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any}>
        <SessionsPane />
      </AppProviders>,
      root,
    );

    const deleteButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("Delete"));
    expect(deleteButton).not.toBeUndefined();
    deleteButton?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await Promise.resolve();
    await Promise.resolve();

    expect(api.deleteSession).toHaveBeenCalledWith("sess-1");
    expect(refresh).toHaveBeenCalled();
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("terminal-owned session"));
  });

  it("duplicates a session with its launch settings and selects the new broker pid", async () => {
    const { api } = await import("../../lib/api");
    const sessionsStore = createStaticStore({
      items: [
        {
          session_id: "sess-1",
          alias: "Inbox cleanup",
          cwd: "/tmp/project",
          agent_backend: "codex",
          provider_choice: "openai-api",
          model: "gpt-5.4",
          reasoning_effort: "high",
          service_tier: "fast",
          transport: "tmux",
        },
      ],
      activeSessionId: "sess-1",
      loading: false,
      tmuxAvailable: true,
      recentCwds: ["/tmp/project"],
      newSessionDefaults: null,
    });

    sessionsStore.refresh = vi.fn(async () => {
      sessionsStore.setState({
        ...sessionsStore.getState(),
        items: [
          ...sessionsStore.getState().items,
          {
            session_id: "sess-2",
            alias: "Inbox cleanup copy",
            cwd: "/tmp/project",
            agent_backend: "codex",
            broker_pid: 42,
          },
        ],
      });
    });

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any}>
        <SessionsPane />
      </AppProviders>,
      root,
    );

    const duplicateButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("Duplicate"));
    expect(duplicateButton).not.toBeUndefined();

    duplicateButton?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await flush();

    expect(api.createSession).toHaveBeenCalledWith({
      cwd: "/tmp/project",
      backend: "codex",
      model: "gpt-5.4",
      model_provider: "openai",
      preferred_auth_method: "apikey",
      reasoning_effort: "high",
      service_tier: "fast",
      create_in_tmux: true,
    });
    expect(sessionsStore.select).toHaveBeenCalledWith("sess-2");
  });

  it("opens the edit dialog and saves the legacy sidebar fields", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionsStore = createStaticStore({
      items: [
        {
          session_id: "sess-1",
          alias: "Inbox cleanup",
          first_user_message: "整理一下今天的会话",
          agent_backend: "pi",
          priority_offset: 0,
        },
        {
          session_id: "sess-2",
          alias: "Release prep",
          agent_backend: "pi",
        },
      ],
      activeSessionId: "sess-1",
      loading: false,
      newSessionDefaults: null,
      recentCwds: [],
      tmuxAvailable: false,
    });
    sessionsStore.refresh = refresh;

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any}>
        <SessionsPane />
      </AppProviders>,
      root,
    );

    const editButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("Edit"));
    expect(editButton).not.toBeUndefined();
    editButton?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await flush();

    const nameInput = root.querySelector('input[name="sessionName"]') as HTMLInputElement;
    expect(nameInput?.value).toBe("Inbox cleanup");

    const dependencySelect = root.querySelector('select[name="dependencySessionId"]') as HTMLSelectElement;
    await act(async () => {
      dependencySelect.value = "sess-2";
      dependencySelect.dispatchEvent(new Event("change", { bubbles: true }));
    });

    const saveButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("Save changes"));
    expect(saveButton).not.toBeUndefined();
    saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await flush();

    expect(api.editSession).toHaveBeenCalledWith("sess-1", {
      name: "Inbox cleanup",
      priority_offset: 0,
      snooze_until: null,
      dependency_session_id: "sess-2",
    });
    expect(refresh).toHaveBeenCalled();
  });
});
