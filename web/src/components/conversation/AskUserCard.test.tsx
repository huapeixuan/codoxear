import { render } from "preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppProviders } from "../../app/providers";
import { AskUserCard } from "./AskUserCard";

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

describe("AskUserCard", () => {
  let root: HTMLDivElement | null = null;

  afterEach(() => {
    vi.clearAllMocks();
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
  });

  it("submits the live ui request id when the message event only has a tool call id", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-ask",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "ui-req-1",
            method: "select",
            question: "Choose a provider",
            context: "Pick one option.",
            options: ["OpenAI", "Anthropic"],
            allow_freeform: true,
            allow_multiple: false,
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            tool_call_id: "tool-call-1",
            question: "Choose a provider",
            context: "Pick one option.",
            options: ["OpenAI", "Anthropic"],
            allow_freeform: true,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-ask"
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    const optionButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("OpenAI")) as
      | HTMLButtonElement
      | undefined;
    expect(optionButton).toBeDefined();

    optionButton?.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledWith("sess-ask", {
      id: "ui-req-1",
      value: "OpenAI",
    });
    expect(refresh).toHaveBeenCalledWith("sess-ask", { agentBackend: "pi" });
  });

  it("matches live pi ui requests that only expose a combined title and custom-response placeholder", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-ask-live",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "40a06318-c9eb-4399-a3ea-6aca99eb55e4",
            method: "select",
            title: "这是 Ask_user 测试，请任选一个选项。\n\nContext:\n你刚刚要求“测试下 Ask_user”。我将发一个最小可用的单题交互，确认工具链正常。",
            options: ["选项 A", "选项 B", "✏️ Type custom response..."],
            allow_freeform: true,
            allow_multiple: false,
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            tool_call_id: "call_3mgq5q5jD7syqr38lHMpFt4t|fc_demo",
            question: "这是 Ask_user 测试，请任选一个选项。",
            context: "你刚刚要求“测试下 Ask_user”。我将发一个最小可用的单题交互，确认工具链正常。",
            options: [
              { title: "选项 A", description: "返回一个固定选项" },
              { title: "选项 B", description: "返回另一个固定选项" },
            ],
            allow_freeform: true,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-ask-live"
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    const optionButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("选项 A")) as
      | HTMLButtonElement
      | undefined;
    expect(optionButton).toBeDefined();

    optionButton?.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledWith("sess-ask-live", {
      id: "40a06318-c9eb-4399-a3ea-6aca99eb55e4",
      value: "选项 A",
    });
    expect(refresh).toHaveBeenCalledWith("sess-ask-live", { agentBackend: "pi" });
  });

  it("prefers live request option values over historical display labels", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-live-values",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "ui-req-values",
            method: "select",
            question: "Choose a provider",
            context: "Pick one option.",
            options: [{ title: "OpenAI", value: "openai" }, { title: "Anthropic", value: "anthropic" }],
            allow_freeform: false,
            allow_multiple: false,
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            tool_call_id: "tool-call-values",
            question: "Choose a provider",
            context: "Pick one option.",
            options: [{ title: "OpenAI" }, { title: "Anthropic" }],
            allow_freeform: false,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-live-values"
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    const optionButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("OpenAI")) as
      | HTMLButtonElement
      | undefined;
    expect(optionButton).toBeDefined();

    optionButton?.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledWith("sess-live-values", {
      id: "ui-req-values",
      value: "openai",
    });
    expect(refresh).toHaveBeenCalledWith("sess-live-values", { agentBackend: "pi" });
  });

  it("does not submit historical-only unresolved ask_user cards without a live ui request", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-history-only",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            tool_call_id: "historic-call-1",
            question: "Choose a provider",
            context: "This prompt only exists in history.",
            options: ["OpenAI", "Anthropic"],
            allow_freeform: true,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-history-only"
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    const optionButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("OpenAI")) as
      | HTMLButtonElement
      | undefined;
    expect(optionButton).toBeDefined();
    expect(optionButton?.disabled).toBe(true);

    optionButton?.click();
    await flush();

    expect(api.submitUiResponse).not.toHaveBeenCalled();
    expect(refresh).not.toHaveBeenCalled();
  });

  it("submits historical-only unresolved ask_user cards when legacy fallback is enabled", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-legacy-fallback",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            tool_call_id: "legacy-call-1",
            question: "Choose a provider",
            context: "Legacy sessions answer through prompt fallback.",
            options: ["OpenAI", "Anthropic"],
            allow_freeform: true,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-legacy-fallback"
          allowLegacyFallback={true}
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    const optionButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("OpenAI")) as
      | HTMLButtonElement
      | undefined;
    expect(optionButton).toBeDefined();
    expect(optionButton?.disabled).toBe(false);

    optionButton?.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledWith("sess-legacy-fallback", {
      id: "legacy-call-1",
      value: "OpenAI",
    });
    expect(refresh).toHaveBeenCalledWith("sess-legacy-fallback", { agentBackend: "pi" });
  });

  it("submits a custom freeform answer for single-select prompts", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-freeform",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "ui-req-freeform",
            method: "select",
            question: "Choose or type one.",
            context: "Custom answers are allowed.",
            options: ["OpenAI", "Anthropic"],
            allow_freeform: true,
            allow_multiple: false,
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            question: "Choose or type one.",
            context: "Custom answers are allowed.",
            options: ["OpenAI", "Anthropic"],
            allow_freeform: true,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-freeform"
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    const textarea = root.querySelector("textarea") as HTMLTextAreaElement | null;
    expect(textarea).not.toBeNull();
    textarea!.value = "Custom provider";
    textarea!.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();

    const submitButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("Submit answer"));
    expect(submitButton).toBeDefined();

    submitButton?.click();
    await flush();

    expect(api.submitUiResponse).toHaveBeenCalledWith("sess-freeform", {
      id: "ui-req-freeform",
      value: "Custom provider",
    });
    expect(refresh).toHaveBeenCalledWith("sess-freeform", { agentBackend: "pi" });
  });

  it("does not bind a historical card to an unrelated lone live request", async () => {
    const { api } = await import("../../lib/api");
    const refresh = vi.fn().mockResolvedValue(undefined);
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-mismatch",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "ui-req-other",
            method: "select",
            question: "Different question",
            context: "Different context",
            options: ["One", "Two"],
            allow_freeform: false,
            allow_multiple: false,
          },
        ],
      },
      { refresh },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            tool_call_id: "historic-mismatch",
            question: "Choose a provider",
            context: "This prompt only exists in history.",
            options: ["OpenAI", "Anthropic"],
            allow_freeform: true,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-mismatch"
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    const optionButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent?.includes("OpenAI")) as
      | HTMLButtonElement
      | undefined;
    expect(optionButton).toBeDefined();
    expect(optionButton?.disabled).toBe(true);

    optionButton?.click();
    await flush();

    expect(api.submitUiResponse).not.toHaveBeenCalled();
    expect(refresh).not.toHaveBeenCalled();
  });

  it("uses live confirm request flags over historical defaults", async () => {
    const sessionUiStore = createStaticStore(
      {
        sessionId: "sess-confirm",
        diagnostics: null,
        queue: null,
        files: [],
        loading: false,
        requests: [
          {
            id: "ui-confirm-1",
            method: "confirm",
            title: "Proceed?\n\nContext:\nLive confirm request.",
            allow_freeform: false,
            allow_multiple: false,
            options: [],
          },
        ],
      },
      { refresh: vi.fn().mockResolvedValue(undefined) },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionUiStore={sessionUiStore as any}>
        <AskUserCard
          event={{
            type: "ask_user",
            tool_call_id: "ui-confirm-1",
            question: "Proceed?",
            context: "Historical defaults should not win.",
            allow_freeform: true,
            allow_multiple: false,
            resolved: false,
          }}
          sessionId="sess-confirm"
          renderRichText={(value: string, className?: string) => <div className={className}>{value}</div>}
        />
      </AppProviders>,
      root,
    );

    expect(root.querySelector("textarea")).toBeNull();
    expect(root.textContent).toContain("Live confirm request.");
  });
});
