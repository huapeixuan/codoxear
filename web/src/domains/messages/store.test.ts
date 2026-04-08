import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../lib/api";
import { createMessagesStore } from "./store";

vi.mock("../../lib/api", () => ({
  api: {
    listMessages: vi.fn(),
  },
}));

describe("createMessagesStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("loads initial messages for a session and clears loading state", async () => {
    vi.mocked(api.listMessages).mockResolvedValue({
      events: [{ id: "m1" }, { id: "m2" }],
      offset: 2,
    } as never);
    const store = createMessagesStore();
    const snapshots: Array<Record<string, unknown>> = [];

    store.subscribe(() => {
      const state = store.getState();
      snapshots.push({
        loading: state.loading,
        messages: state.bySessionId.s1 ?? [],
      });
    });

    await store.loadInitial("s1");

    expect(api.listMessages).toHaveBeenCalledWith("s1", true, undefined, undefined);
    expect(snapshots).toEqual([
      { loading: true, messages: [] },
      { loading: false, messages: [{ id: "m1" }, { id: "m2" }] },
    ]);
    expect(store.getState()).toEqual({
      bySessionId: {
        s1: [{ id: "m1" }, { id: "m2" }],
      },
      offsetsBySessionId: {
        s1: 2,
      },
      loading: false,
    });
  });

  it("replaces messages for a polled session without touching other sessions", async () => {
    vi.mocked(api.listMessages)
      .mockResolvedValueOnce({
        events: [{ id: "m1" }],
        offset: 1,
      } as never)
      .mockResolvedValueOnce({
        events: [{ id: "other" }],
        offset: 1,
      } as never)
      .mockResolvedValueOnce({
        events: [{ id: "m2" }],
        offset: 2,
      } as never);
    const store = createMessagesStore();

    await store.loadInitial("s1");
    await store.loadInitial("s2");
    await store.poll("s1");

    expect(api.listMessages).toHaveBeenNthCalledWith(1, "s1", true, undefined, undefined);
    expect(api.listMessages).toHaveBeenNthCalledWith(2, "s2", true, undefined, undefined);
    expect(api.listMessages).toHaveBeenNthCalledWith(3, "s1", false, undefined, 1);
    expect(store.getState()).toEqual({
      bySessionId: {
        s1: [{ id: "m1" }, { id: "m2" }],
        s2: [{ id: "other" }],
      },
      offsetsBySessionId: {
        s1: 2,
        s2: 1,
      },
      loading: false,
    });
  });

  it("clears loading when message fetch fails", async () => {
    vi.mocked(api.listMessages).mockRejectedValue(new Error("boom"));
    const store = createMessagesStore();

    await expect(store.loadInitial("s1")).rejects.toThrow("boom");
    expect(store.getState()).toEqual({
      bySessionId: {},
      offsetsBySessionId: {},
      loading: false,
    });
  });

  it("ignores stale message responses", async () => {
    let resolveFirst: (v: any) => void;
    let resolveSecond: (v: any) => void;
    vi.mocked(api.listMessages)
      .mockReturnValueOnce(new Promise((r) => { resolveFirst = r; }))
      .mockReturnValueOnce(new Promise((r) => { resolveSecond = r; }));
    const store = createMessagesStore();

    const firstPromise = store.poll("s1");
    const secondPromise = store.poll("s1");

    resolveSecond!({ events: [{ id: "m2" }] });
    await secondPromise;
    expect(store.getState().bySessionId.s1).toEqual([{ id: "m2" }]);

    resolveFirst!({ events: [{ id: "m1" }] });
    await firstPromise;
    expect(store.getState().bySessionId.s1).toEqual([{ id: "m2" }]);
  });

  it("passes the saved offset when polling", async () => {
    vi.mocked(api.listMessages)
      .mockResolvedValueOnce({ events: [{ id: "m1" }], offset: 4 } as never)
      .mockResolvedValueOnce({ events: [{ id: "m2" }], offset: 5 } as never);
    const store = createMessagesStore();

    await store.loadInitial("s1");
    await store.poll("s1");

    expect(api.listMessages).toHaveBeenNthCalledWith(2, "s1", false, undefined, 4);
    expect(store.getState().bySessionId.s1).toEqual([{ id: "m1" }, { id: "m2" }]);
  });
});
