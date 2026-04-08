import { getJson, postJson } from "./http";
import type {
  AudioListenerResponse,
  CreateSessionResponse,
  MessagesResponse,
  NotificationFeedResponse,
  NotificationMessageResponse,
  NotificationSubscriptionStateResponse,
  RenameSessionResponse,
  SessionFileListResponse,
  SessionResumeCandidatesResponse,
  SessionUiStateResponse,
  SessionsResponse,
  VoiceSettingsResponse,
} from "./types";

export const api = {
  listSessions(signal?: AbortSignal) {
    return getJson<SessionsResponse>("/api/sessions", signal);
  },
  listMessages(sessionId: string, init = false, signal?: AbortSignal, offset?: number) {
    const query = new URLSearchParams();
    if (init) {
      query.set("init", "1");
    }
    if (typeof offset === "number" && Number.isFinite(offset) && offset > 0) {
      query.set("offset", String(offset));
    }
    const suffix = query.size ? `?${query.toString()}` : "";
    return getJson<MessagesResponse>(`/api/sessions/${sessionId}/messages${suffix}`, signal);
  },
  getSessionUiState(sessionId: string, signal?: AbortSignal) {
    return getJson<SessionUiStateResponse>(`/api/sessions/${sessionId}/ui_state`, signal);
  },
  async sendMessage(sessionId: string, text: string) {
    return postJson(`/api/sessions/${sessionId}/send`, { text });
  },
  async createSession(payload: Record<string, unknown>) {
    return postJson<CreateSessionResponse>(`/api/sessions`, payload);
  },
  getSessionResumeCandidates(cwd: string, agentBackend: string) {
    const query = new URLSearchParams();
    query.set("cwd", cwd);
    query.set("backend", agentBackend);
    query.set("agent_backend", agentBackend);
    return getJson<SessionResumeCandidatesResponse>(`/api/session_resume_candidates?${query.toString()}`);
  },
  renameSession(sessionId: string, name: string) {
    return postJson<RenameSessionResponse>(`/api/sessions/${sessionId}/rename`, { name });
  },
  getVoiceSettings() {
    return getJson<VoiceSettingsResponse>(`/api/settings/voice`);
  },
  saveVoiceSettings(payload: Record<string, unknown>) {
    return postJson<VoiceSettingsResponse>(`/api/settings/voice`, payload);
  },
  setAudioListener(clientId: string, enabled: boolean) {
    return postJson<AudioListenerResponse>(`/api/audio/listener`, { client_id: clientId, enabled });
  },
  getNotificationsFeed(since: number) {
    const query = new URLSearchParams();
    query.set("since", String(since));
    return getJson<NotificationFeedResponse>(`/api/notifications/feed?${query.toString()}`);
  },
  getNotificationMessage(messageId: string) {
    const query = new URLSearchParams();
    query.set("message_id", messageId);
    return getJson<NotificationMessageResponse>(`/api/notifications/message?${query.toString()}`);
  },
  getNotificationSubscriptionState() {
    return getJson<NotificationSubscriptionStateResponse>(`/api/notifications/subscription`);
  },
  upsertNotificationSubscription(payload: Record<string, unknown>) {
    return postJson<NotificationSubscriptionStateResponse>(`/api/notifications/subscription`, payload);
  },
  toggleNotificationSubscription(endpoint: string, enabled: boolean) {
    return postJson<NotificationSubscriptionStateResponse>(`/api/notifications/subscription/toggle`, { endpoint, enabled });
  },
  getDiagnostics(sessionId: string) {
    return getJson(`/api/sessions/${sessionId}/diagnostics`);
  },
  getQueue(sessionId: string) {
    return getJson(`/api/sessions/${sessionId}/queue`);
  },
  getFiles(sessionId: string) {
    return getJson<SessionFileListResponse>(`/api/sessions/${sessionId}/file/list`);
  },
  submitUiResponse(sessionId: string, payload: Record<string, unknown>) {
    return postJson(`/api/sessions/${sessionId}/ui_response`, payload);
  },
};
