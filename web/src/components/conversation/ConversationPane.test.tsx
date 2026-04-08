import { render } from "preact";
import { act } from "preact/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppProviders } from "../../app/providers";
import { ConversationPane } from "./ConversationPane";

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

describe("ConversationPane", () => {
  let root: HTMLDivElement | null = null;

  afterEach(() => {
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
  });

  it("renders user, assistant, and ask_user events with card surfaces", () => {
    const sessionsStore = createStaticStore(
      { items: [], activeSessionId: "sess-1", loading: false, newSessionDefaults: null },
      { refresh: () => Promise.resolve(), select: () => undefined },
    );
    const messagesStore = createStaticStore(
      {
        bySessionId: {
          "sess-1": [
            { role: "user", text: "Hello" },
            { type: "ask_user", question: "Choose a provider", answer: "openai", resolved: true },
            { type: "ask_user", question: "Need anything else?", cancelled: true, resolved: true },
            { role: "assistant", text: "All done." },
          ],
        },
        offsetsBySessionId: { "sess-1": 3 },
        loading: false,
      },
      { loadInitial: () => Promise.resolve(), poll: () => Promise.resolve() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any} messagesStore={messagesStore as any}>
        <ConversationPane />
      </AppProviders>,
      root,
    );

    const text = root.textContent || "";
    expect(root.querySelectorAll("[data-testid='message-surface']").length).toBeGreaterThanOrEqual(4);
    expect(root.querySelector("[data-testid='message-surface'][data-kind='user']")).not.toBeNull();
    expect(root.querySelector("[data-testid='message-surface'][data-kind='assistant']")).not.toBeNull();
    expect(root.querySelector("[data-testid='message-surface'][data-kind='ask_user']")).not.toBeNull();
    expect(text).toContain("Hello");
    expect(text).toContain("Choose a provider");
    expect(text).toContain("Answer: openai");
    expect(text).toContain("Need anything else?");
    expect(text).toContain("Cancelled");
    expect(text).toContain("All done.");
  });

  it("renders legacy-visible activity events in the main conversation flow", () => {
    const sessionsStore = createStaticStore(
      { items: [], activeSessionId: "sess-2", loading: false, newSessionDefaults: null },
      { refresh: () => Promise.resolve(), select: () => undefined },
    );
    const messagesStore = createStaticStore(
      {
        bySessionId: {
          "sess-2": [
            { role: "user", text: "Please fix this" },
            { type: "reasoning", text: "thinking...", summary: "Inspecting current state" },
            { type: "tool", name: "read" },
            { type: "tool_result", name: "read", text: "{\"ok\":true}" },
            { type: "subagent", agent: "reviewer", task: "Check the patch", output: "Looks good" },
            {
              type: "todo_snapshot",
              progress_text: "2/3 completed",
              items: [
                { title: "Explore project context", status: "completed" },
                { title: "Render message types", status: "in-progress" },
              ],
            },
            { type: "pi_model_change", summary: "Switched to gpt-5.4" },
            { role: "assistant", text: "Fixed." },
          ],
        },
        offsetsBySessionId: { "sess-2": 7 },
        loading: false,
      },
      { loadInitial: () => Promise.resolve(), poll: () => Promise.resolve() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any} messagesStore={messagesStore as any}>
        <ConversationPane />
      </AppProviders>,
      root,
    );

    const text = root.textContent || "";
    expect(root.querySelectorAll(".messageRow")).toHaveLength(8);
    expect(root.querySelector("[data-testid='message-surface'][data-kind='reasoning']")).not.toBeNull();
    expect(root.querySelector("[data-testid='message-surface'][data-kind='tool']")).not.toBeNull();
    expect(root.querySelector("[data-testid='message-surface'][data-kind='tool_result']")).not.toBeNull();
    expect(root.querySelector("[data-testid='message-surface'][data-kind='subagent']")).not.toBeNull();
    expect(root.querySelector("[data-testid='message-surface'][data-kind='todo_snapshot']")).not.toBeNull();
    expect(root.querySelector("[data-testid='message-surface'][data-kind='pi_model_change']")).not.toBeNull();
    expect(text).toContain("Please fix this");
    expect(text).toContain("thinking...");
    expect(text).toContain("Inspecting current state");
    expect(text).toContain("read");
    expect(text).toContain('{"ok":true}');
    expect(text).toContain("reviewer");
    expect(text).toContain("Check the patch");
    expect(text).toContain("Looks good");
    expect(text).toContain("2/3 completed");
    expect(text).toContain("Switched to gpt-5.4");
    expect(text).toContain("Fixed.");
  });

  it("groups consecutive assistant messages and avoids showing role labels as body text", () => {
    const sessionsStore = createStaticStore(
      { items: [], activeSessionId: "sess-3", loading: false, newSessionDefaults: null },
      { refresh: () => Promise.resolve(), select: () => undefined },
    );
    const messagesStore = createStaticStore(
      {
        bySessionId: {
          "sess-3": [
            { role: "assistant", text: "First answer" },
            { role: "assistant", text: "Second answer" },
            { role: "user", text: "Thanks" },
          ],
        },
        offsetsBySessionId: { "sess-3": 3 },
        loading: false,
      },
      { loadInitial: () => Promise.resolve(), poll: () => Promise.resolve() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any} messagesStore={messagesStore as any}>
        <ConversationPane />
      </AppProviders>,
      root,
    );

    expect(root.querySelectorAll(".messageRow")).toHaveLength(3);
    expect(root.querySelectorAll(".messageRow.grouped")).toHaveLength(1);
    expect(root.textContent).not.toContain("assistantassistant");
  });

  it("renders markdown code blocks and inline code inside assistant messages", () => {
    const sessionsStore = createStaticStore(
      { items: [], activeSessionId: "sess-4", loading: false, newSessionDefaults: null },
      { refresh: () => Promise.resolve(), select: () => undefined },
    );
    const messagesStore = createStaticStore(
      {
        bySessionId: {
          "sess-4": [
            {
              role: "assistant",
              text: "Run `npm test` before `<div>` output.\n\n```ts\nconst answer = 42;\n```",
            },
          ],
        },
        offsetsBySessionId: { "sess-4": 1 },
        loading: false,
      },
      { loadInitial: () => Promise.resolve(), poll: () => Promise.resolve() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any} messagesStore={messagesStore as any}>
        <ConversationPane />
      </AppProviders>,
      root,
    );

    const inlineCode = Array.from(root.querySelectorAll(".messageBody p code")).map((node) => node.textContent || "");
    const blockCode = root.querySelector(".messageBody pre code");
    expect(inlineCode).toContain("npm test");
    expect(inlineCode).toContain("<div>");
    expect(blockCode?.textContent).toContain("const answer = 42;");
  });

  it("resets expandable message state when switching sessions", async () => {
    const longReasoningA = Array.from({ length: 10 }, (_value, index) => `session A reasoning ${index + 1}`).join("\n");
    const longReasoningB = Array.from({ length: 10 }, (_value, index) => `session B reasoning ${index + 1}`).join("\n");
    const messagesStore = createStaticStore(
      {
        bySessionId: {
          "sess-expand-a": [{ type: "reasoning", text: longReasoningA }],
          "sess-expand-b": [{ type: "reasoning", text: longReasoningB }],
        },
        offsetsBySessionId: { "sess-expand-a": 1, "sess-expand-b": 1 },
        loading: false,
      },
      { loadInitial: () => Promise.resolve(), poll: () => Promise.resolve() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders
        sessionsStore={createStaticStore(
          { items: [], activeSessionId: "sess-expand-a", loading: false, newSessionDefaults: null },
          { refresh: () => Promise.resolve(), select: () => undefined },
        ) as any}
        messagesStore={messagesStore as any}
      >
        <ConversationPane />
      </AppProviders>,
      root,
    );

    const firstButton = root.querySelector("[data-testid='message-surface'][data-kind='reasoning'] .messageExpandButton") as HTMLButtonElement | null;
    const firstContent = root.querySelector("[data-testid='message-surface'][data-kind='reasoning'] .messageExpandableContent") as HTMLDivElement | null;

    expect(firstButton?.getAttribute("aria-expanded")).toBe("false");
    await act(async () => {
      firstButton?.click();
      await Promise.resolve();
    });
    expect(firstButton?.getAttribute("aria-expanded")).toBe("true");
    expect(firstContent?.classList.contains("isCollapsed")).toBe(false);

    await act(async () => {
      render(
        <AppProviders
          sessionsStore={createStaticStore(
            { items: [], activeSessionId: "sess-expand-b", loading: false, newSessionDefaults: null },
            { refresh: () => Promise.resolve(), select: () => undefined },
          ) as any}
          messagesStore={messagesStore as any}
        >
          <ConversationPane />
        </AppProviders>,
        root!,
      );
      await Promise.resolve();
    });

    const nextButton = root.querySelector("[data-testid='message-surface'][data-kind='reasoning'] .messageExpandButton") as HTMLButtonElement | null;
    const nextContent = root.querySelector("[data-testid='message-surface'][data-kind='reasoning'] .messageExpandableContent") as HTMLDivElement | null;

    expect(nextButton?.getAttribute("aria-expanded")).toBe("false");
    expect(nextContent?.classList.contains("isCollapsed")).toBe(true);
    expect(root.textContent).toContain("session B reasoning 1");
  });

  it("collapses long reasoning and tool-result cards with expandable toggles", async () => {
    const sessionsStore = createStaticStore(
      { items: [], activeSessionId: "sess-6", loading: false, newSessionDefaults: null },
      { refresh: () => Promise.resolve(), select: () => undefined },
    );
    const longReasoning = Array.from({ length: 10 }, (_value, index) => `reasoning line ${index + 1}`).join("\n");
    const longToolResult = Array.from({ length: 12 }, (_value, index) => `result line ${index + 1}`).join("\n");
    const messagesStore = createStaticStore(
      {
        bySessionId: {
          "sess-6": [
            { type: "reasoning", text: longReasoning },
            { type: "tool_result", name: "bash", text: longToolResult },
          ],
        },
        offsetsBySessionId: { "sess-6": 2 },
        loading: false,
      },
      { loadInitial: () => Promise.resolve(), poll: () => Promise.resolve() },
    );

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any} messagesStore={messagesStore as any}>
        <ConversationPane />
      </AppProviders>,
      root,
    );

    const reasoningButton = root.querySelector("[data-testid='message-surface'][data-kind='reasoning'] .messageExpandButton") as HTMLButtonElement | null;
    const reasoningContent = root.querySelector("[data-testid='message-surface'][data-kind='reasoning'] .messageExpandableContent") as HTMLDivElement | null;
    const toolButton = root.querySelector("[data-testid='message-surface'][data-kind='tool_result'] .messageExpandButton") as HTMLButtonElement | null;
    const toolContent = root.querySelector("[data-testid='message-surface'][data-kind='tool_result'] .messageExpandableContent") as HTMLDivElement | null;

    expect(reasoningButton?.textContent).toBe("Show more");
    expect(reasoningButton?.getAttribute("aria-expanded")).toBe("false");
    expect(reasoningContent?.classList.contains("isCollapsed")).toBe(true);
    expect(toolButton?.textContent).toBe("Show more");
    expect(toolButton?.getAttribute("aria-expanded")).toBe("false");
    expect(toolContent?.classList.contains("isCollapsed")).toBe(true);

    reasoningButton?.click();
    toolButton?.click();
    await Promise.resolve();

    expect(reasoningButton?.textContent).toBe("Show less");
    expect(reasoningButton?.getAttribute("aria-expanded")).toBe("true");
    expect(reasoningContent?.classList.contains("isCollapsed")).toBe(false);
    expect(toolButton?.textContent).toBe("Show less");
    expect(toolButton?.getAttribute("aria-expanded")).toBe("true");
    expect(toolContent?.classList.contains("isCollapsed")).toBe(false);
  });

  it("scrolls the conversation pane to the bottom on initial render when messages exist", async () => {
    const sessionsStore = createStaticStore(
      { items: [], activeSessionId: "sess-7", loading: false, newSessionDefaults: null },
      { refresh: () => Promise.resolve(), select: () => undefined },
    );
    const messagesStore = createStaticStore(
      {
        bySessionId: {
          "sess-7": [
            { role: "assistant", text: "One" },
            { role: "assistant", text: "Two" },
            { role: "assistant", text: "Three" },
          ],
        },
        offsetsBySessionId: { "sess-7": 3 },
        loading: false,
      },
      { loadInitial: () => Promise.resolve(), poll: () => Promise.resolve() },
    );

    const scrollTo = vi.fn();
    const originalScrollTo = HTMLElement.prototype.scrollTo;
    HTMLElement.prototype.scrollTo = scrollTo;
    const scrollHeightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
    const clientHeightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
      configurable: true,
      get() {
        return this.classList?.contains("conversationPane") ? 500 : (scrollHeightDescriptor?.get?.call(this) ?? 0);
      },
    });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get() {
        return this.classList?.contains("conversationPane") ? 200 : (clientHeightDescriptor?.get?.call(this) ?? 0);
      },
    });

    root = document.createElement("div");
    document.body.appendChild(root);
    render(
      <AppProviders sessionsStore={sessionsStore as any} messagesStore={messagesStore as any}>
        <ConversationPane />
      </AppProviders>,
      root,
    );

    await Promise.resolve();
    await Promise.resolve();

    expect(scrollTo).toHaveBeenCalledWith({ top: 500 });
    HTMLElement.prototype.scrollTo = originalScrollTo;

    if (scrollHeightDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "scrollHeight", scrollHeightDescriptor);
    }
    if (clientHeightDescriptor) {
      Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeightDescriptor);
    }
  });
});
