import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../lib/api";
import { createComposerStore } from "./store";

vi.mock("../../lib/api", () => ({
  api: {
    sendMessage: vi.fn(),
  },
}));

describe("createComposerStore", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("keeps drafts scoped per session", () => {
    const store = createComposerStore();

    store.setDraft("s1", "/aut");
    store.setDraft("s2", "hello");

    expect(store.getState().draftBySessionId).toEqual({
      s1: "/aut",
      s2: "hello",
    });
  });

  it("clears sending state after a successful submit", async () => {
    vi.mocked(api.sendMessage).mockResolvedValue({ ok: true } as never);
    const store = createComposerStore();
    store.setDraft("s1", "hello");

    await store.submit("s1");

    expect(api.sendMessage).toHaveBeenCalledWith("s1", "hello", undefined);
    expect(store.getState()).toEqual({
      draftBySessionId: {
        s1: "",
      },
      sending: false,
      pendingBySessionId: {
        s1: [
          {
            localId: "local-pending-1",
            role: "user",
            text: "hello",
            pending: true,
          },
        ],
      },
    });
  });

  it("restores sending=false on failure without clearing draft", async () => {
    vi.mocked(api.sendMessage).mockRejectedValue(new Error("fail"));
    const store = createComposerStore();
    store.setDraft("s1", "keep me");

    await expect(store.submit("s1")).rejects.toThrow("fail");

    expect(store.getState()).toEqual({ draftBySessionId: { s1: "keep me" }, sending: false, pendingBySessionId: { s1: [] } });
  });

  it("adds an optimistic pending message immediately and keeps it until persistence catches up", async () => {
    let resolveSend: (value: unknown) => void = () => undefined;
    vi.mocked(api.sendMessage).mockReturnValueOnce(new Promise((resolve) => {
      resolveSend = resolve;
    }) as never);
    const store = createComposerStore();
    store.setDraft("s1", "hello");

    const submitPromise = store.submit("s1");

    expect(store.getState().draftBySessionId.s1).toBe("");
    expect(store.getState().sending).toBe(true);
    expect(store.getState().pendingBySessionId.s1).toHaveLength(1);
    expect(store.getState().pendingBySessionId.s1[0]).toMatchObject({ role: "user", text: "hello", pending: true });

    resolveSend({ ok: true });
    await submitPromise;

    expect(store.getState().draftBySessionId.s1).toBe("");
    expect(store.getState().sending).toBe(false);
    expect(store.getState().pendingBySessionId.s1).toHaveLength(1);
  });

  it("removes the optimistic pending message and restores the draft after failure", async () => {
    let rejectSend: (error?: unknown) => void = () => undefined;
    vi.mocked(api.sendMessage).mockReturnValueOnce(new Promise((_resolve, reject) => {
      rejectSend = reject;
    }) as never);
    const store = createComposerStore();
    store.setDraft("s1", "keep me");

    const submitPromise = store.submit("s1");

    expect(store.getState().draftBySessionId.s1).toBe("");
    expect(store.getState().pendingBySessionId.s1).toHaveLength(1);

    rejectSend(new Error("fail"));
    await expect(submitPromise).rejects.toThrow("fail");

    expect(store.getState()).toEqual({ draftBySessionId: { s1: "keep me" }, sending: false, pendingBySessionId: { s1: [] } });
  });

  it("clears acknowledged pending messages when persisted user messages arrive", async () => {
    vi.mocked(api.sendMessage).mockResolvedValue({ ok: true } as never);
    const store = createComposerStore();
    store.setDraft("s1", "hello");

    await store.submit("s1");
    expect(store.getState().pendingBySessionId.s1).toHaveLength(1);

    store.clearAcknowledgedPending("s1", [
      { role: "assistant", text: "working" },
      { role: "user", text: "hello" },
    ]);

    expect(store.getState()).toEqual({ draftBySessionId: { s1: "" }, sending: false, pendingBySessionId: { s1: [] } });
  });

  it("forwards optional image payloads when submitting", async () => {
    vi.mocked(api.sendMessage).mockResolvedValue({ ok: true } as never);
    const store = createComposerStore();
    store.setDraft("s1", "hello");

    await store.submit("s1", {
      images: [{
        file_name: "shot.png",
        mime_type: "image/png",
        data_b64: "aGVsbG8=",
      }],
    });

    expect(api.sendMessage).toHaveBeenCalledWith("s1", "hello", {
      images: [{
        file_name: "shot.png",
        mime_type: "image/png",
        data_b64: "aGVsbG8=",
      }],
    });
  });
});
