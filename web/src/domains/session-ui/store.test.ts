import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../lib/api";
import { createSessionUiStore } from "./store";

vi.mock("../../lib/api", () => ({
  api: {
    getSessionUiState: vi.fn(),
    getDiagnostics: vi.fn(),
    getQueue: vi.fn(),
    getFiles: vi.fn(),
  },
}));

describe("createSessionUiStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  function createDeferred<T>() {
    let resolve!: (value: T | PromiseLike<T>) => void;
    let reject!: (reason?: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  }

  it("refreshes all workspace panels and emits loading transitions", async () => {
    vi.mocked(api.getSessionUiState).mockResolvedValue({ requests: [{ id: "r1" }] });
    vi.mocked(api.getDiagnostics).mockResolvedValue({ status: "ok" });
    vi.mocked(api.getQueue).mockResolvedValue({ items: [] });
    vi.mocked(api.getFiles).mockResolvedValue({ files: ["f1.py"] } as any);

    const store = createSessionUiStore();
    await store.refresh("s1", { agentBackend: "pi" });

    expect(api.getSessionUiState).toHaveBeenCalledWith("s1");
    expect(store.getState()).toEqual({
      sessionId: "s1",
      requests: [{ id: "r1" }],
      diagnostics: { status: "ok" },
      queue: { items: [] },
      files: ["f1.py"],
      loading: false,
    });
  });

  it("skips ui_state fetches for non-pi sessions", async () => {
    vi.mocked(api.getDiagnostics).mockResolvedValue({ status: "ok" });
    vi.mocked(api.getQueue).mockResolvedValue({ items: [] });
    vi.mocked(api.getFiles).mockResolvedValue({ files: ["f1.py"] } as any);

    const store = createSessionUiStore();
    await store.refresh("s1", { agentBackend: "codex" });

    expect(api.getSessionUiState).not.toHaveBeenCalled();
    expect(store.getState().requests).toEqual([]);
  });

  it("keeps same-session workspace data visible while a refresh is in flight", async () => {
    vi.mocked(api.getSessionUiState).mockResolvedValueOnce({ requests: [{ id: "r1" }] });
    vi.mocked(api.getDiagnostics).mockResolvedValueOnce({ todo_snapshot: { progress_text: "1/2 completed" } });
    vi.mocked(api.getQueue).mockResolvedValueOnce({ items: [] });
    vi.mocked(api.getFiles).mockResolvedValueOnce({ files: ["todo.md"] } as any);

    const nextUiState = createDeferred<{ requests: Array<{ id: string }> }>();
    const nextDiagnostics = createDeferred<Record<string, unknown>>();
    const nextQueue = createDeferred<Record<string, unknown>>();
    const nextFiles = createDeferred<{ files: string[] }>();

    vi.mocked(api.getSessionUiState).mockReturnValueOnce(nextUiState.promise as any);
    vi.mocked(api.getDiagnostics).mockReturnValueOnce(nextDiagnostics.promise as any);
    vi.mocked(api.getQueue).mockReturnValueOnce(nextQueue.promise as any);
    vi.mocked(api.getFiles).mockReturnValueOnce(nextFiles.promise as any);

    const store = createSessionUiStore();

    await store.refresh("s1", { agentBackend: "pi" });

    const refreshPromise = store.refresh("s1", { agentBackend: "pi" });

    expect(store.getState()).toEqual({
      sessionId: "s1",
      requests: [{ id: "r1" }],
      diagnostics: { todo_snapshot: { progress_text: "1/2 completed" } },
      queue: { items: [] },
      files: ["todo.md"],
      loading: true,
    });

    nextUiState.resolve({ requests: [{ id: "r2" }] });
    nextDiagnostics.resolve({ todo_snapshot: { progress_text: "2/2 completed" } });
    nextQueue.resolve({ items: ["queued"] });
    nextFiles.resolve({ files: ["todo.md", "done.md"] });
    await refreshPromise;

    expect(store.getState()).toEqual({
      sessionId: "s1",
      requests: [{ id: "r2" }],
      diagnostics: { todo_snapshot: { progress_text: "2/2 completed" } },
      queue: { items: ["queued"] },
      files: ["todo.md", "done.md"],
      loading: false,
    });
  });

  it("clears workspace data immediately when switching sessions", async () => {
    vi.mocked(api.getSessionUiState).mockResolvedValueOnce({ requests: [{ id: "r1" }] });
    vi.mocked(api.getDiagnostics).mockResolvedValueOnce({ todo_snapshot: { progress_text: "1/2 completed" } });
    vi.mocked(api.getQueue).mockResolvedValueOnce({ items: [] });
    vi.mocked(api.getFiles).mockResolvedValueOnce({ files: ["todo.md"] } as any);

    const nextUiState = createDeferred<{ requests: Array<{ id: string }> }>();
    const nextDiagnostics = createDeferred<Record<string, unknown>>();
    const nextQueue = createDeferred<Record<string, unknown>>();
    const nextFiles = createDeferred<{ files: string[] }>();

    vi.mocked(api.getSessionUiState).mockReturnValueOnce(nextUiState.promise as any);
    vi.mocked(api.getDiagnostics).mockReturnValueOnce(nextDiagnostics.promise as any);
    vi.mocked(api.getQueue).mockReturnValueOnce(nextQueue.promise as any);
    vi.mocked(api.getFiles).mockReturnValueOnce(nextFiles.promise as any);

    const store = createSessionUiStore();

    await store.refresh("s1", { agentBackend: "pi" });

    const refreshPromise = store.refresh("s2", { agentBackend: "pi" });

    expect(store.getState()).toEqual({
      sessionId: "s2",
      requests: [],
      diagnostics: null,
      queue: null,
      files: [],
      loading: true,
    });

    nextUiState.resolve({ requests: [{ id: "r2" }] });
    nextDiagnostics.resolve({ todo_snapshot: { progress_text: "0/1 completed" } });
    nextQueue.resolve({ items: [] });
    nextFiles.resolve({ files: ["fresh.md"] });
    await refreshPromise;

    expect(store.getState()).toEqual({
      sessionId: "s2",
      requests: [{ id: "r2" }],
      diagnostics: { todo_snapshot: { progress_text: "0/1 completed" } },
      queue: { items: [] },
      files: ["fresh.md"],
      loading: false,
    });
  });
});
