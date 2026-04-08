import { render } from "preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppProviders } from "../../app/providers";
import { SessionWorkspace } from "./SessionWorkspace";

vi.mock("../../lib/api", () => ({
  api: {
    submitUiResponse: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
}

function createStaticStore<TState extends object, TActions extends Record<string, (...args: any[]) => any>>(
  state: TState,
  actions: TActions,
) {
  return {
    getState: () => state,
    subscribe: () => () => undefined,
    ...actions,
  };
}

describe("SessionWorkspace", () => {
  let root: HTMLDivElement | null = null;

  afterEach(() => {
    vi.clearAllMocks();
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
  });

  it("renders structured ask_user prompts and submits multi-select with freeform input", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-1",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "req-1",
            method: "select",
            question: "Choose deployment targets",
            context: "You can pick more than one.",
            allow_multiple: true,
            allow_freeform: true,
            options: [
              { title: "Alpha", description: "Primary region" },
              { title: "Beta", description: "Backup region" },
            ],
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <SessionWorkspace />
      </AppProviders>,
      root,
    );

    expect(root.textContent).toContain("Choose deployment targets");
    expect(root.textContent).toContain("You can pick more than one.");
    expect(root.textContent).toContain("Alpha");
    expect(root.textContent).toContain("Primary region");

    const checkboxes = root.querySelectorAll('input[type="checkbox"]');
    const alphaCheckbox = checkboxes[0] as HTMLInputElement;
    alphaCheckbox.click();
    await flush();

    const freeform = root.querySelector('textarea[placeholder="Other response"]') as HTMLTextAreaElement;
    freeform.value = "Gamma";
    freeform.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();

    const confirm = Array.from(root.querySelectorAll("button")).find((button) => button.textContent === "Confirm") as HTMLButtonElement;
    confirm.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledWith("sess-1", {
      id: "req-1",
      value: ["Alpha", "Gamma"],
    });
    expect(refresh).toHaveBeenCalledWith("sess-1", { agentBackend: "pi" });
  });

  it("locks request actions while submitting and shows an error when submission fails", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    let rejectRequest: (error?: unknown) => void = () => undefined;
    const pendingSubmission = new Promise((_resolve, reject) => {
      rejectRequest = reject;
    });
    vi.mocked(api.submitUiResponse).mockReturnValueOnce(pendingSubmission as any);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-error",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "req-error",
            method: "confirm",
            question: "Continue with deploy?",
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <SessionWorkspace />
      </AppProviders>,
      root,
    );

    const confirm = Array.from(root.querySelectorAll("button")).find((button) => button.textContent === "Confirm") as HTMLButtonElement;
    confirm.click();
    confirm.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledTimes(1);
    expect(confirm.disabled).toBe(true);
    expect(root.textContent).toContain("Submitting...");

    rejectRequest(new Error("Broker unavailable"));
    await flush();

    expect(root.textContent).toContain("Broker unavailable");
    expect(refresh).not.toHaveBeenCalled();
    expect(confirm.disabled).toBe(false);
  });

  it("uses the first option as the default single-select answer", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-2",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "req-2",
            method: "select",
            question: "Choose a model",
            options: [
              { title: "fast", description: "Quickest option" },
              { title: "balanced", description: "Default option" },
            ],
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <SessionWorkspace />
      </AppProviders>,
      root,
    );

    const confirm = Array.from(root.querySelectorAll("button")).find((button) => button.textContent === "Confirm") as HTMLButtonElement;
    confirm.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledWith("sess-2", {
      id: "req-2",
      value: "fast",
    });
  });

  it("renders only user-facing request content instead of diagnostics-heavy panels", () => {
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-1",
        diagnostics: { status: "ok" },
        queue: { items: [{ text: "next task" }] },
        files: ["src/main.tsx"],
        loading: false,
        requests: [
          {
            id: "req-3",
            method: "confirm",
            question: "Continue with deploy?",
            context: "Need explicit confirmation.",
          },
        ],
      },
      { refresh: vi.fn() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <SessionWorkspace />
      </AppProviders>,
      root,
    );

    expect(root.querySelector('[data-testid="workspace-card"]')).not.toBeNull();
    expect(root.querySelectorAll('[data-testid="workspace-tab"]').length).toBeGreaterThanOrEqual(3);
    expect(root.textContent).toContain("UI Requests");
    expect(root.textContent).toContain("Continue with deploy?");
    expect(root.textContent).toContain("Diagnostics");
    expect(root.textContent).toContain("Queue");
  });

  it("can render diagnostics in details mode when explicitly requested", () => {
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-9",
        diagnostics: { status: "ok", queue_len: 2 },
        queue: { items: [{ text: "next task" }] },
        files: ["src/main.tsx"],
        loading: false,
        requests: [],
      },
      { refresh: vi.fn() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <SessionWorkspace mode="details" />
      </AppProviders>,
      root,
    );

    expect(root.textContent).toContain("Diagnostics");
    expect(root.textContent).toContain("status");
    expect(root.textContent).toContain("queue_len");
    expect(root.textContent).toContain("next task");
    expect(root.textContent).toContain("src/main.tsx");
  });
});
