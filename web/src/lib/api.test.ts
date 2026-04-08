import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { getJson } from "./http";
import type { MessagesResponse, SessionUiStateResponse, SessionsResponse } from "./types";

describe("getJson", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("throws server error messages", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 400,
        text: async () => '{"error":"bad request"}',
      }),
    );

    await expect(getJson("/api/sessions")).rejects.toThrow("bad request");
  });

  it("parses successful json responses", async () => {
    const payload: SessionsResponse = {
      sessions: [{ session_id: "s1", agent_backend: "pi", busy: true }],
      new_session_defaults: { default_backend: "pi" },
    };
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        text: async () => JSON.stringify(payload),
      }),
    );

    await expect(getJson<SessionsResponse>("/api/sessions")).resolves.toEqual(payload);
  });
});

describe("api", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("requests sessions with the provided abort signal", async () => {
    const signal = new AbortController().signal;
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"sessions":[]}',
    });
    vi.stubGlobal("fetch", fetchMock);

    const payload = await api.listSessions(signal);

    expect(payload).toEqual({ sessions: [] });
    expect(fetchMock).toHaveBeenCalledWith("api/sessions", {
      headers: { Accept: "application/json" },
      signal,
    });
  });

  it("builds the init messages route", async () => {
    const payload: MessagesResponse = { events: [], offset: 0, ui_version: "v1" };
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => JSON.stringify(payload),
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.listMessages("session-1", true)).resolves.toEqual(payload);
    expect(fetchMock).toHaveBeenCalledWith("api/sessions/session-1/messages?init=1", {
      headers: { Accept: "application/json" },
      signal: undefined,
    });
  });

  it("includes offsets when polling messages", async () => {
    const payload: MessagesResponse = { events: [{ id: "m1" }], offset: 9 };
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => JSON.stringify(payload),
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.listMessages("session-1", false, undefined, 4)).resolves.toEqual(payload);
    expect(fetchMock).toHaveBeenCalledWith("api/sessions/session-1/messages?offset=4", {
      headers: { Accept: "application/json" },
      signal: undefined,
    });
  });

  it("requests the session ui state", async () => {
    const payload: SessionUiStateResponse = {
      requests: [{ id: "ui-req-1", method: "select" }],
    };
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => JSON.stringify(payload),
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.getSessionUiState("pi-session")).resolves.toEqual(payload);
    expect(fetchMock).toHaveBeenCalledWith("api/sessions/pi-session/ui_state", {
      headers: { Accept: "application/json" },
      signal: undefined,
    });
  });

  it("requests voice settings", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"ok":true,"tts_base_url":"https://example.test/v1"}',
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.getVoiceSettings()).resolves.toEqual({ ok: true, tts_base_url: "https://example.test/v1" });
    expect(fetchMock).toHaveBeenCalledWith("api/settings/voice", {
      headers: { Accept: "application/json" },
      signal: undefined,
    });
  });

  it("posts announcement listener heartbeats", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"ok":true,"active_listener_count":1}',
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.setAudioListener("listener-1", true)).resolves.toEqual({ ok: true, active_listener_count: 1 });
    expect(fetchMock).toHaveBeenCalledWith("api/audio/listener", expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ client_id: "listener-1", enabled: true }),
    }));
  });

  it("requests the notification feed with a since cursor", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"ok":true,"items":[]}',
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.getNotificationsFeed(123.5)).resolves.toEqual({ ok: true, items: [] });
    expect(fetchMock).toHaveBeenCalledWith("api/notifications/feed?since=123.5", {
      headers: { Accept: "application/json" },
      signal: undefined,
    });
  });

  it("requests notification message details by id", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"ok":true,"notification_text":"ready"}',
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.getNotificationMessage("msg-1")).resolves.toEqual({ ok: true, notification_text: "ready" });
    expect(fetchMock).toHaveBeenCalledWith("api/notifications/message?message_id=msg-1", {
      headers: { Accept: "application/json" },
      signal: undefined,
    });
  });

  it("requests notification subscription state", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"ok":true,"subscriptions":[]}',
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.getNotificationSubscriptionState()).resolves.toEqual({ ok: true, subscriptions: [] });
    expect(fetchMock).toHaveBeenCalledWith("api/notifications/subscription", {
      headers: { Accept: "application/json" },
      signal: undefined,
    });
  });

  it("posts notification subscriptions", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"ok":true,"subscriptions":[]}',
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.upsertNotificationSubscription({ subscription: { endpoint: "https://push.test/sub/1" } })).resolves.toEqual({ ok: true, subscriptions: [] });
    expect(fetchMock).toHaveBeenCalledWith("api/notifications/subscription", expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ subscription: { endpoint: "https://push.test/sub/1" } }),
    }));
  });

  it("toggles notification subscriptions by endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: async () => '{"ok":true,"subscriptions":[]}',
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.toggleNotificationSubscription("https://push.test/sub/1", false)).resolves.toEqual({ ok: true, subscriptions: [] });
    expect(fetchMock).toHaveBeenCalledWith("api/notifications/subscription/toggle", expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ endpoint: "https://push.test/sub/1", enabled: false }),
    }));
  });
});
