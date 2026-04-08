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

  it("clears sending state after a successful submit", async () => {
    vi.mocked(api.sendMessage).mockResolvedValue({ ok: true } as never);
    const store = createComposerStore();
    store.setDraft("hello");

    await store.submit("s1");

    expect(api.sendMessage).toHaveBeenCalledWith("s1", "hello");
    expect(store.getState()).toEqual({ draft: "", sending: false });
  });

  it("restores sending=false on failure without clearing draft", async () => {
    vi.mocked(api.sendMessage).mockRejectedValue(new Error("fail"));
    const store = createComposerStore();
    store.setDraft("keep me");

    await expect(store.submit("s1")).rejects.toThrow("fail");

    expect(store.getState()).toEqual({ draft: "keep me", sending: false });
  });
});
