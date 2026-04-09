import { render } from "preact";
import { act } from "preact/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppProviders } from "../../app/providers";

const { todoPanelRenderLog } = vi.hoisted(() => ({
  todoPanelRenderLog: [] as Array<{ expanded: boolean; progressText: unknown }>,
}));

function getLoggedProgressText(snapshot: unknown) {
  if (!snapshot || typeof snapshot !== "object") {
    return null;
  }

  return (snapshot as Record<string, unknown>).progress_text ?? null;
}

vi.mock("./TodoComposerPanel", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./TodoComposerPanel")>();

  return {
    ...actual,
    TodoComposerPanel: (props: Parameters<typeof actual.TodoComposerPanel>[0]) => {
      todoPanelRenderLog.push({ expanded: props.expanded, progressText: getLoggedProgressText(props.snapshot) });
      return <actual.TodoComposerPanel {...props} />;
    },
  };
});

import { Composer } from "./Composer";

interface RenderComposerOptions {
  activeSessionId?: string | null;
  items?: Array<{ session_id: string; agent_backend: string; busy: boolean }>;
  sessionUiSessionId?: string | null;
  diagnostics?: Record<string, unknown> | null;
  draft?: string;
}

let root: HTMLDivElement | null = null;

function getRoot() {
  if (!root) {
    throw new Error("Composer test root has not been initialized");
  }

  return root;
}

function createStore<TState extends object, TActions extends Record<string, (...args: any[]) => any>>(
  initialState: TState,
  actionFactory: (setState: (patch: Partial<TState>) => void, getState: () => TState) => TActions,
) {
  let state = initialState;
  const listeners = new Set<() => void>();
  const setState = (patch: Partial<TState>) => {
    state = { ...state, ...patch };
    listeners.forEach((listener) => listener());
  };
  const getState = () => state;
  return {
    getState,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    ...actionFactory(setState, getState),
  };
}

function renderComposer(options: RenderComposerOptions = {}) {
  const {
    activeSessionId = "sess-1",
    items = [{ session_id: "sess-1", agent_backend: "pi", busy: false }],
    sessionUiSessionId = activeSessionId,
    diagnostics = null,
    draft = "Hello",
  } = options;
  const submit = vi.fn().mockResolvedValue(undefined);
  const sessionsStore = createStore(
    { items, activeSessionId, loading: false, newSessionDefaults: null },
    (setState) => ({ refresh: vi.fn(), select: vi.fn(), setState }),
  );
  const composerStore = createStore(
    { draft, sending: false },
    (setState) => ({
      setDraft(value: string) {
        setState({ draft: value });
      },
      submit,
    }),
  );
  const sessionUiStore = createStore(
    {
      sessionId: sessionUiSessionId,
      diagnostics,
      queue: null,
      files: [],
      requests: [],
      loading: false,
    },
    (setState) => ({ refresh: vi.fn(), setState }),
  );

  root = document.createElement("div");
  document.body.appendChild(root);
  render(
    <AppProviders sessionsStore={sessionsStore as any} composerStore={composerStore as any} sessionUiStore={sessionUiStore as any}>
      <Composer />
    </AppProviders>,
    root,
  );

  return { submit, sessionsStore, composerStore, sessionUiStore };
}

describe("Composer", () => {
  afterEach(() => {
    window.localStorage.clear();
    todoPanelRenderLog.length = 0;
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
  });

  it("submits on plain Enter when enter-to-send is enabled", async () => {
    window.localStorage.setItem("codoxear.enterToSend", "1");
    const { submit } = renderComposer({ items: [] });
    const composerRoot = getRoot();

    const textarea = composerRoot.querySelector("textarea") as HTMLTextAreaElement;
    expect(composerRoot.querySelector("[data-testid='composer-card']")).not.toBeNull();
    expect(composerRoot.querySelector("button[type='submit']")).not.toBeNull();
    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    Object.defineProperty(event, "isComposing", { value: false });
    textarea.dispatchEvent(event);

    expect(submit).toHaveBeenCalledWith("sess-1");
    expect(event.defaultPrevented).toBe(true);
  });

  it("submits on ctrl+enter when enter-to-send is disabled", () => {
    const { submit } = renderComposer({ items: [] });
    const composerRoot = getRoot();

    const textarea = composerRoot.querySelector("textarea") as HTMLTextAreaElement;
    const event = new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true });
    Object.defineProperty(event, "isComposing", { value: false });
    textarea.dispatchEvent(event);

    expect(submit).toHaveBeenCalledWith("sess-1");
    expect(event.defaultPrevented).toBe(true);
  });

  it("does not submit on plain Enter when enter-to-send is disabled", () => {
    const { submit } = renderComposer({ items: [] });
    const composerRoot = getRoot();

    const textarea = composerRoot.querySelector("textarea") as HTMLTextAreaElement;
    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    Object.defineProperty(event, "isComposing", { value: false });
    textarea.dispatchEvent(event);

    expect(submit).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("shows a todo summary bar above the composer for a current pi session with todo items", () => {
    renderComposer({
      diagnostics: {
        todo_snapshot: {
          available: true,
          error: false,
          progress_text: "2/3 completed",
          items: [{ title: "Move todo above composer", status: "in-progress" }],
        },
      },
    });
    const composerRoot = getRoot();

    expect(composerRoot.querySelector(".composerTodoBar")).not.toBeNull();
    expect(composerRoot.querySelector(".composerTodoBar + [data-testid='composer-card']")).not.toBeNull();
    expect(composerRoot.querySelector(".composerTodoBar")?.nextElementSibling?.getAttribute("data-testid")).toBe("composer-card");
    expect(composerRoot.textContent).toContain("2/3 completed");
  });

  it("expands and collapses the todo panel when the summary bar is clicked", async () => {
    renderComposer({
      diagnostics: {
        todo_snapshot: {
          available: true,
          error: false,
          progress_text: "1/2 completed",
          items: [{ title: "Keep toggle wired", status: "in-progress" }],
        },
      },
    });
    const composerRoot = getRoot();

    const toggle = composerRoot.querySelector(".composerTodoBarButton") as HTMLButtonElement | null;

    expect(toggle).not.toBeNull();
    expect(composerRoot.querySelector(".composerTodoPanel")).toBeNull();

    toggle?.click();
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();

    toggle?.click();
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoPanel")).toBeNull();
  });

  it("does not show a todo bar for non-pi sessions", () => {
    renderComposer({
      items: [{ session_id: "sess-1", agent_backend: "codex", busy: false }],
      diagnostics: {
        todo_snapshot: {
          available: true,
          error: false,
          progress_text: "1/1 completed",
          items: [{ title: "Should stay hidden", status: "completed" }],
        },
      },
    });
    const composerRoot = getRoot();

    expect(composerRoot.querySelector(".composerTodoBar")).toBeNull();
  });

  it("does not show a todo bar when session ui state is stale", () => {
    renderComposer({
      sessionUiSessionId: "sess-2",
      diagnostics: {
        todo_snapshot: {
          available: true,
          error: false,
          progress_text: "1/1 completed",
          items: [{ title: "Should stay hidden", status: "completed" }],
        },
      },
    });
    const composerRoot = getRoot();

    expect(composerRoot.querySelector(".composerTodoBar")).toBeNull();
  });

  it("does not show a todo bar when the snapshot has no valid items", () => {
    renderComposer({
      diagnostics: {
        todo_snapshot: {
          available: true,
          error: false,
          progress_text: "1/1 completed",
          items: [{ title: "   ", status: "   ", description: "   " }],
        },
      },
    });
    const composerRoot = getRoot();

    expect(composerRoot.querySelector(".composerTodoBar")).toBeNull();
  });

  it("preserves expansion when a session switch temporarily hides the todo bar", async () => {
    const { sessionsStore } = renderComposer({
      items: [
        { session_id: "sess-1", agent_backend: "pi", busy: false },
        { session_id: "sess-2", agent_backend: "pi", busy: false },
      ],
      diagnostics: {
        todo_snapshot: {
          available: true,
          error: false,
          progress_text: "1/2 completed",
          items: [{ title: "Reset when switching", status: "in-progress" }],
        },
      },
    });
    const composerRoot = getRoot();

    (composerRoot.querySelector(".composerTodoBarButton") as HTMLButtonElement | null)?.click();
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();

    (sessionsStore as any).setState({ activeSessionId: "sess-2" });
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoBar")).toBeNull();

    (sessionsStore as any).setState({ activeSessionId: "sess-1" });
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoBar")).not.toBeNull();
    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();
  });

  it("restores the todo panel expanded after same-session diagnostics temporarily hide it", async () => {
    const snapshot = {
      available: true,
      error: false,
      progress_text: "1/2 completed",
      items: [{ title: "Keep expansion through refresh", status: "in-progress" }],
    };
    const { sessionUiStore } = renderComposer({
      diagnostics: {
        todo_snapshot: snapshot,
      },
    });
    const composerRoot = getRoot();

    (composerRoot.querySelector(".composerTodoBarButton") as HTMLButtonElement | null)?.click();
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();

    act(() => {
      (sessionUiStore as any).setState({
        sessionId: "sess-1",
        diagnostics: null,
      });
    });

    expect(composerRoot.querySelector(".composerTodoBar")).toBeNull();

    act(() => {
      (sessionUiStore as any).setState({
        sessionId: "sess-1",
        diagnostics: { todo_snapshot: snapshot },
      });
    });

    expect(composerRoot.querySelector(".composerTodoBar")).not.toBeNull();
    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();
  });

  it("remembers expanded state separately for each pi session when switching", async () => {
    const firstSnapshot = {
      available: true,
      error: false,
      progress_text: "1/2 completed",
      items: [{ title: "Open source session todo", status: "in-progress" }],
    };
    const secondSnapshot = {
      available: true,
      error: false,
      progress_text: "2/3 completed",
      items: [{ title: "Destination session todo", status: "in-progress" }],
    };
    const { sessionsStore, sessionUiStore } = renderComposer({
      items: [
        { session_id: "sess-1", agent_backend: "pi", busy: false },
        { session_id: "sess-2", agent_backend: "pi", busy: false },
      ],
      diagnostics: {
        todo_snapshot: firstSnapshot,
      },
    });
    const composerRoot = getRoot();

    (composerRoot.querySelector(".composerTodoBarButton") as HTMLButtonElement | null)?.click();
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();

    const renderCountBeforeSwitch = todoPanelRenderLog.length;

    act(() => {
      (sessionUiStore as any).setState({
        sessionId: "sess-2",
        diagnostics: { todo_snapshot: secondSnapshot },
      });
      (sessionsStore as any).setState({ activeSessionId: "sess-2" });
    });

    const destinationRenders = todoPanelRenderLog
      .slice(renderCountBeforeSwitch)
      .filter((entry) => entry.progressText === "2/3 completed");

    expect(destinationRenders[0]?.expanded).toBe(false);
    expect(composerRoot.querySelector(".composerTodoBar")).not.toBeNull();
    expect(composerRoot.textContent).toContain("2/3 completed");
    expect(composerRoot.querySelector(".composerTodoPanel")).toBeNull();

    (composerRoot.querySelector(".composerTodoBarButton") as HTMLButtonElement | null)?.click();
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();

    act(() => {
      (sessionUiStore as any).setState({
        sessionId: "sess-1",
        diagnostics: { todo_snapshot: firstSnapshot },
      });
      (sessionsStore as any).setState({ activeSessionId: "sess-1" });
    });

    expect(composerRoot.textContent).toContain("1/2 completed");
    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();

    act(() => {
      (sessionUiStore as any).setState({
        sessionId: "sess-2",
        diagnostics: { todo_snapshot: secondSnapshot },
      });
      (sessionsStore as any).setState({ activeSessionId: "sess-2" });
    });

    expect(composerRoot.textContent).toContain("2/3 completed");
    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();
  });

  it("expands on the first click after switching directly between displayable pi sessions", async () => {
    const firstSnapshot = {
      available: true,
      error: false,
      progress_text: "1/2 completed",
      items: [{ title: "Open source session todo", status: "in-progress" }],
    };
    const secondSnapshot = {
      available: true,
      error: false,
      progress_text: "2/3 completed",
      items: [{ title: "Destination session todo", status: "in-progress" }],
    };
    const { sessionsStore, sessionUiStore } = renderComposer({
      items: [
        { session_id: "sess-1", agent_backend: "pi", busy: false },
        { session_id: "sess-2", agent_backend: "pi", busy: false },
      ],
      diagnostics: {
        todo_snapshot: firstSnapshot,
      },
    });
    const composerRoot = getRoot();

    (composerRoot.querySelector(".composerTodoBarButton") as HTMLButtonElement | null)?.click();
    await Promise.resolve();

    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();

    act(() => {
      (sessionUiStore as any).setState({
        sessionId: "sess-2",
        diagnostics: { todo_snapshot: secondSnapshot },
      });
      (sessionsStore as any).setState({ activeSessionId: "sess-2" });
      (composerRoot.querySelector(".composerTodoBarButton") as HTMLButtonElement | null)?.click();
    });

    expect(composerRoot.textContent).toContain("2/3 completed");
    expect(composerRoot.querySelector(".composerTodoPanel")).not.toBeNull();
  });
});
