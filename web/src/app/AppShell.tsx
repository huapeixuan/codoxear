import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { api } from "../lib/api";
import type { NotificationSubscriptionStateResponse, VoiceSettingsResponse } from "../lib/types";
import { SessionsPane } from "../components/sessions/SessionsPane";
import { ConversationPane } from "../components/conversation/ConversationPane";
import { Composer } from "../components/composer/Composer";
import { SessionWorkspace } from "../components/workspace/SessionWorkspace";
import { FileViewerDialog } from "../components/workspace/FileViewerDialog";
import type { FileViewMode } from "../components/workspace/FileViewerDialog";
import { HarnessDialog } from "../components/workspace/HarnessDialog";
import { NewSessionDialog } from "../components/new-session/NewSessionDialog";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Sheet, SheetContent } from "@/components/ui/sheet";
import { toPublicAssetUrl } from "../lib/publicAssetUrl";
import { useMessagesStore, useMessagesStoreApi, useSessionUiStore, useSessionUiStoreApi, useSessionsStore, useSessionsStoreApi } from "./providers";

function shortSessionId(sessionId: string) {
  const match = sessionId.match(/^([0-9a-f]{8})[0-9a-f-]{20,}$/i);
  return match ? match[1] : sessionId.slice(0, 8);
}

function readLocalToggle(key: string) {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem(key) === "1";
}

function writeLocalToggle(key: string, value: boolean) {
  if (typeof window === "undefined") return;
  if (value) {
    window.localStorage.setItem(key, "1");
  } else {
    window.localStorage.removeItem(key);
  }
}

function readLocalToggleDefaultOn(key: string) {
  if (typeof window === "undefined") return true;
  return window.localStorage.getItem(key) !== "0";
}

function getAnnouncementClientId() {
  if (typeof window === "undefined") return "announcement-client";
  const key = "codoxear.announcementClientId";
  const current = window.localStorage.getItem(key);
  if (current) return current;
  const next = typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `announcement-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  window.localStorage.setItem(key, next);
  return next;
}

function base64UrlToUint8Array(value: string) {
  const raw = String(value || "");
  const pad = "=".repeat((4 - (raw.length % 4 || 4)) % 4);
  const base64 = (raw + pad).replace(/-/g, "+").replace(/_/g, "/");
  const data = atob(base64);
  const out = new Uint8Array(data.length);
  for (let index = 0; index < data.length; index += 1) {
    out[index] = data.charCodeAt(index);
  }
  return out;
}

function isMobileNotificationDevice() {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent || "";
  if (/Android|iPhone|iPad|iPod|Mobile/i.test(ua)) return true;
  if (/Macintosh/i.test(ua) && Number(navigator.maxTouchPoints || 0) > 1) return true;
  return false;
}

function notificationDeviceClass() {
  return isMobileNotificationDevice() ? "mobile" : "desktop";
}

function shouldUseMobileWorkspaceSheet() {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
  return window.matchMedia("(max-width: 880px)").matches;
}

function shouldPreferNativeHlsPlayback() {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent || "";
  const vendor = navigator.vendor || "";
  const isAppleVendor = /Apple/i.test(vendor);
  const isSafariEngine = /Safari/i.test(ua) && !/Chrom(e|ium)|Edg|OPR|CriOS|FxiOS/i.test(ua);
  return isAppleVendor || isSafariEngine;
}

function BellIcon() {
  return (
    <svg className="actionIcon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <path d="M15 17H5l2-2v-4a5 5 0 1 1 10 0v4l2 2h-4" />
      <path d="M10 17a2 2 0 0 0 4 0" />
    </svg>
  );
}

function VolumeIcon() {
  return (
    <svg className="actionIcon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <path d="M11 5L6 9H3v6h3l5 4V5z" />
      <path d="M15 9a5 5 0 0 1 0 6" />
      <path d="M18.5 6.5a9 9 0 0 1 0 11" />
    </svg>
  );
}

function MoreIcon() {
  return (
    <svg className="actionIcon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
      <circle cx="5" cy="12" r="1.5" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none" />
      <circle cx="19" cy="12" r="1.5" fill="currentColor" stroke="none" />
    </svg>
  );
}

const DEFAULT_VOICE_SETTINGS: VoiceSettingsResponse = {
  tts_enabled_for_narration: false,
  tts_enabled_for_final_response: true,
  tts_base_url: "",
  tts_api_key: "",
  audio: {
    active_listener_count: 0,
    queue_depth: 0,
    segment_count: 0,
    stream_url: "/api/audio/live.m3u8",
  },
  notifications: {
    enabled_devices: 0,
    total_devices: 0,
    vapid_public_key: "",
  },
};

const NOTIFICATION_MESSAGE_RETRY_MS = 15000;
const FINAL_NOTIFICATION_SUMMARY_STATUSES = new Set(["sent", "skipped", "error"]);

type NotificationMessageLookupState = {
  retryAfter: number;
  terminal: boolean;
};

function mergeVoiceSettings(value: VoiceSettingsResponse | null | undefined): VoiceSettingsResponse {
  return {
    ...DEFAULT_VOICE_SETTINGS,
    ...(value || {}),
    audio: {
      ...DEFAULT_VOICE_SETTINGS.audio,
      ...((value && value.audio) || {}),
    },
    notifications: {
      ...DEFAULT_VOICE_SETTINGS.notifications,
      ...((value && value.notifications) || {}),
    },
  };
}

function EmptyDetailsWorkspace() {
  return (
    <aside className="workspacePane">
      <section className="workspaceSection">
        <h3>Diagnostics</h3>
        <p>No diagnostics available.</p>
      </section>
      <section className="workspaceSection">
        <h3>Queue</h3>
        <ul className="workspaceList">
          <li>No queued items</li>
        </ul>
      </section>
      <section className="workspaceSection">
        <h3>Files</h3>
        <ul className="workspaceList">
          <li>No tracked files</li>
        </ul>
      </section>
      <section className="workspaceSection">
        <h3>UI Requests</h3>
        <p>No pending requests</p>
      </section>
    </aside>
  );
}

export function AppShell() {
  const { bySessionId } = useMessagesStore();
  const { activeSessionId, items } = useSessionsStore();
  const { sessionId: sessionUiSessionId, files } = useSessionUiStore();
  const sessionsStoreApi = useSessionsStoreApi();
  const messagesStoreApi = useMessagesStoreApi();
  const sessionUiStoreApi = useSessionUiStoreApi();
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const [mobileToolsOpen, setMobileToolsOpen] = useState(false);
  const [fileViewerOpen, setFileViewerOpen] = useState(false);
  const [harnessOpen, setHarnessOpen] = useState(false);
  const [fileViewerPath, setFileViewerPath] = useState("");
  const [fileViewerLine, setFileViewerLine] = useState<number | null>(null);
  const [fileViewerMode, setFileViewerMode] = useState<FileViewMode | null>(null);
  const [fileViewerRequestKey, setFileViewerRequestKey] = useState(0);
  const [voiceSettingsOpen, setVoiceSettingsOpen] = useState(false);
  const [voiceSettingsStatus, setVoiceSettingsStatus] = useState("");
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [announcementEnabled, setAnnouncementEnabled] = useState(() => readLocalToggle("codoxear.announcementEnabled"));
  const [notificationsEnabled, setNotificationsEnabled] = useState(() => readLocalToggle("codoxear.notificationEnabled"));
  const [replySoundEnabled, setReplySoundEnabled] = useState(() => readLocalToggleDefaultOn("codoxear.replySoundEnabled"));
  const [voiceSettings, setVoiceSettings] = useState<VoiceSettingsResponse>(DEFAULT_VOICE_SETTINGS);
  const [voiceBaseUrlDraft, setVoiceBaseUrlDraft] = useState("");
  const [voiceApiKeyDraft, setVoiceApiKeyDraft] = useState("");
  const [narrationEnabledDraft, setNarrationEnabledDraft] = useState(false);
  const [enterToSendDraft, setEnterToSendDraft] = useState(() => readLocalToggle("codoxear.enterToSend"));
  const [notificationPermission, setNotificationPermission] = useState(() => (
    typeof Notification === "undefined" ? "unsupported" : Notification.permission
  ));
  const liveAudioRef = useRef<HTMLAudioElement | null>(null);
  const audioRetryTimerRef = useRef<number | null>(null);
  const hlsRef = useRef<any>(null);
  const notificationFeedCursorRef = useRef(Date.now() / 1000);
  const deliveredNotificationIdsRef = useRef(new Set<string>());
  const resolvingNotificationIdsRef = useRef(new Set<string>());
  const notificationLookupStateRef = useRef(new Map<string, NotificationMessageLookupState>());
  const notificationEndpointRef = useRef("");
  const seenFinalResponseKeysRef = useRef(new Set<string>());
  const finalResponseBeepPrimedRef = useRef(false);
  const announcementClientId = useMemo(() => getAnnouncementClientId(), []);
  const [pushNotificationsEnabled, setPushNotificationsEnabled] = useState(false);

  useEffect(() => {
    sessionsStoreApi.refresh().catch(() => undefined);
    const intervalId = window.setInterval(() => {
      sessionsStoreApi.refresh().catch(() => undefined);
    }, 3000);
    return () => window.clearInterval(intervalId);
  }, [sessionsStoreApi]);

  useEffect(() => {
    let cancelled = false;
    api.getVoiceSettings().then((response) => {
      if (cancelled) return;
      const nextSettings = mergeVoiceSettings(response);
      setVoiceSettings(nextSettings);
      setVoiceBaseUrlDraft(String(nextSettings.tts_base_url || ""));
      setVoiceApiKeyDraft(String(nextSettings.tts_api_key || ""));
      setNarrationEnabledDraft(!!nextSettings.tts_enabled_for_narration);
      if (announcementEnabled) {
        queueMicrotask(() => startAnnouncementPlayback(nextSettings));
      }
    }).catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    writeLocalToggle("codoxear.announcementEnabled", announcementEnabled);
  }, [announcementEnabled]);

  useEffect(() => {
    writeLocalToggle("codoxear.notificationEnabled", notificationsEnabled);
  }, [notificationsEnabled]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem("codoxear.replySoundEnabled", replySoundEnabled ? "1" : "0");
  }, [replySoundEnabled]);

  useEffect(() => {
    let intervalId: number | undefined;
    api.setAudioListener(announcementClientId, announcementEnabled).catch(() => undefined);
    if (announcementEnabled) {
      intervalId = window.setInterval(() => {
        api.setAudioListener(announcementClientId, true).catch(() => undefined);
      }, 15000);
    }
    return () => {
      if (typeof intervalId === "number") {
        window.clearInterval(intervalId);
      }
      api.setAudioListener(announcementClientId, false).catch(() => undefined);
    };
  }, [announcementClientId, announcementEnabled]);

  const startAnnouncementPlayback = (settings: VoiceSettingsResponse, { resetSource = false, force = false } = {}) => {
    const audio = liveAudioRef.current;
    if (!audio) return;
    if (!force && !announcementEnabled) return;
    const streamUrl = String(settings.audio?.stream_url || "").trim();
    const hasSegments = Number(settings.audio?.segment_count || 0) > 0;

    const browserPrefersNativeHls = shouldPreferNativeHlsPlayback();
    const nativeHls = browserPrefersNativeHls && ["application/vnd.apple.mpegurl", "audio/mpegurl"].some((kind) => {
      const result = audio.canPlayType(kind);
      return result === "probably" || result === "maybe";
    });

    const Hls = (window as any).Hls;
    const canUseHlsJs = Hls && Hls.isSupported();

    if (!streamUrl || !hasSegments || (!nativeHls && !canUseHlsJs)) {
      return;
    }

    if (nativeHls) {
      if (resetSource || audio.src !== streamUrl) {
        console.log("[Audio] Using native HLS playback", streamUrl);
        audio.src = streamUrl;
      }
    } else if (canUseHlsJs) {
      if (!hlsRef.current) {
        console.log("[Audio] Initializing hls.js");
        const hls = new Hls({
          enableWorker: true,
          lowLatencyMode: true,
          backBufferLength: 30,
          manifestLoadingMaxRetry: 10,
          levelLoadingMaxRetry: 10,
        });
        hls.on(Hls.Events.ERROR, (_event: any, data: any) => {
          console.warn("[Audio] hls.js error:", data);
          if (data.fatal) {
            switch (data.type) {
              case Hls.ErrorTypes.NETWORK_ERROR:
                hls.startLoad();
                break;
              case Hls.ErrorTypes.MEDIA_ERROR:
                hls.recoverMediaError();
                break;
              default:
                void startAnnouncementPlayback(settings, { resetSource: true });
                break;
            }
          }
        });
        hls.attachMedia(audio);
        hlsRef.current = hls;
      }
      const hls = hlsRef.current;
      if (resetSource || audio.getAttribute("data-hls-url") !== streamUrl) {
        console.log("[Audio] Loading HLS source via hls.js", streamUrl);
        audio.setAttribute("data-hls-url", streamUrl);
        hls.loadSource(streamUrl);
      }
    }

    const playPromise = audio.play();
    if (playPromise !== undefined) {
      playPromise.then(() => {
        if (resetSource) console.log("[Audio] Playback started/resumed");
      }).catch((err) => {
        console.warn("[Audio] Playback failed, will retry:", err);
        if (audioRetryTimerRef.current !== null) {
          window.clearTimeout(audioRetryTimerRef.current);
        }
        audioRetryTimerRef.current = window.setTimeout(() => {
          audioRetryTimerRef.current = null;
          startAnnouncementPlayback(settings, { resetSource: true });
        }, 1200);
      });
    }
  };

  useEffect(() => {
    const audio = liveAudioRef.current;
    if (!audio) return;
    if (!announcementEnabled) {
      if (hlsRef.current) {
        hlsRef.current.destroy();
        hlsRef.current = null;
      }
      audio.removeAttribute("src");
      audio.removeAttribute("data-hls-url");
      if (audio.src) {
        audio.src = "";
      }
      if (audioRetryTimerRef.current !== null) {
        window.clearTimeout(audioRetryTimerRef.current);
        audioRetryTimerRef.current = null;
      }
      return;
    }
    startAnnouncementPlayback(voiceSettings);
  }, [announcementEnabled, voiceSettings.audio?.segment_count, voiceSettings.audio?.stream_url]);

  useEffect(() => {
    const audio = liveAudioRef.current;
    if (!audio) return;
    const retry = () => startAnnouncementPlayback(voiceSettings, { resetSource: true });
    audio.addEventListener("ended", retry);
    audio.addEventListener("error", retry);
    return () => {
      audio.removeEventListener("ended", retry);
      audio.removeEventListener("error", retry);
    };
  }, [announcementEnabled, voiceSettings]);

  const ensureVoiceServiceWorker = async () => {
    if (!("serviceWorker" in navigator)) {
      throw new Error("service workers are not available");
    }
    return navigator.serviceWorker.register(toPublicAssetUrl("service-worker.js"));
  };

  const syncNotificationSubscriptionState = async (
    snapshot?: NotificationSubscriptionStateResponse | null,
    endpointOverride?: string,
  ) => {
    if (notificationDeviceClass() !== "mobile" || !("serviceWorker" in navigator) || typeof PushManager === "undefined") {
      notificationEndpointRef.current = "";
      setPushNotificationsEnabled(false);
      return;
    }
    let endpoint = String(endpointOverride || "").trim();
    if (!endpoint) {
      const registration = await ensureVoiceServiceWorker();
      const subscription = await registration.pushManager.getSubscription();
      endpoint = subscription && typeof subscription.endpoint === "string" ? subscription.endpoint : "";
    }
    notificationEndpointRef.current = endpoint;
    const state = snapshot ?? await api.getNotificationSubscriptionState();
    const current = endpoint ? state.subscriptions.find((item) => item && item.endpoint === endpoint) : null;
    setPushNotificationsEnabled(Boolean(current && current.notifications_enabled));
  };

  useEffect(() => {
    if (notificationDeviceClass() !== "mobile") return;
    syncNotificationSubscriptionState().catch(() => undefined);
  }, [voiceSettings.notifications?.vapid_public_key]);

  useEffect(() => {
    if (
      !notificationsEnabled
      || notificationPermission !== "granted"
      || typeof Notification === "undefined"
      || notificationDeviceClass() !== "desktop"
    ) {
      return;
    }

    notificationFeedCursorRef.current = Date.now() / 1000;
    let cancelled = false;

    const showDesktopNotification = (title: string, body: string, messageId?: string) => {
      const trimmedBody = body.replace(/\s+/g, " ").trim();
      if (!trimmedBody) return;
      const id = String(messageId || "").trim();
      if (id && deliveredNotificationIdsRef.current.has(id)) return;
      new Notification(title.trim() || "Session", {
        body: trimmedBody.length <= 180 ? trimmedBody : `${trimmedBody.slice(0, 179).trimEnd()}...`,
        tag: id || `desktop:${Date.now()}`,
      });
      if (id) deliveredNotificationIdsRef.current.add(id);
    };

    const pollFeed = async (prime = false) => {
      const response = await api.getNotificationsFeed(notificationFeedCursorRef.current);
      if (cancelled) return;
      let maxSeen = notificationFeedCursorRef.current;
      for (const item of response.items || []) {
        const updatedTs = Number(item.updated_ts || 0);
        if (updatedTs > maxSeen) maxSeen = updatedTs;
        if (prime) continue;
        showDesktopNotification(
          String(item.session_display_name || "Session"),
          String(item.notification_text || ""),
          item.message_id,
        );
      }
      notificationFeedCursorRef.current = maxSeen;
    };

    void pollFeed(true);
    const intervalId = window.setInterval(() => {
      void pollFeed(false);
    }, 5000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [notificationPermission, notificationsEnabled]);

  useEffect(() => {
    const nextSeen = new Set<string>();
    for (const sessionId of Object.keys(bySessionId)) {
      const events = Array.isArray(bySessionId[sessionId]) ? bySessionId[sessionId] : [];
      for (const event of events) {
        if (!event || typeof event !== "object") continue;
        const row = event as Record<string, unknown>;
        if (row.role !== "assistant" || row.pending === true || row.message_class !== "final_response") continue;
        const key = finalResponseEventKey(row);
        if (!key) continue;
        nextSeen.add(key);
        if (finalResponseBeepPrimedRef.current && replySoundEnabled && !seenFinalResponseKeysRef.current.has(key)) {
          playReplyBeep();
        }
      }
    }
    seenFinalResponseKeysRef.current = nextSeen;
    finalResponseBeepPrimedRef.current = true;
  }, [bySessionId, replySoundEnabled]);

  const activeSession = items.find((session) => session.session_id === activeSessionId) ?? null;
  const sessionUiMatchesActiveSession = !!activeSessionId && sessionUiSessionId === activeSessionId;

  const openFileViewer = (path = "", line: number | null = null, mode: FileViewMode | null = null) => {
    setFileViewerPath(path);
    setFileViewerLine(line);
    setFileViewerMode(mode);
    setFileViewerRequestKey((current) => current + 1);
    setFileViewerOpen(true);
  };

  const closeFileViewer = () => {
    setFileViewerOpen(false);
    setFileViewerPath("");
    setFileViewerLine(null);
    setFileViewerMode(null);
  };

  const logout = async () => {
    try {
      await api.logout();
      if (typeof window !== "undefined") {
        window.location.reload();
      }
    } catch {
      // allow retry from the UI
    }
  };

  useEffect(() => {
    if (!activeSessionId) {
      return;
    }

    messagesStoreApi.loadInitial(activeSessionId).catch(() => undefined);
    sessionUiStoreApi.refresh(activeSessionId, { agentBackend: activeSession?.agent_backend }).catch(() => undefined);
    const intervalId = window.setInterval(() => {
      messagesStoreApi.poll(activeSessionId).catch(() => undefined);
      sessionUiStoreApi.refresh(activeSessionId, { agentBackend: activeSession?.agent_backend }).catch(() => undefined);
    }, 3000);
    return () => window.clearInterval(intervalId);
  }, [activeSession?.agent_backend, activeSessionId, messagesStoreApi, sessionUiStoreApi]);

  useEffect(() => {
    if (activeSessionId) {
      setSidebarOpen(false);
    }
  }, [activeSessionId]);

  useEffect(() => {
    setFileViewerOpen(false);
    setHarnessOpen(false);
  }, [activeSessionId]);

  const activeTitle = activeSession
    ? activeSession.alias || activeSession.first_user_message || activeSession.title || shortSessionId(activeSession.session_id)
    : "No session selected";
  const shellClassName = useMemo(() => ["appShell", "editorialShell"].join(" "), []);
  const hasAnnouncementCredentials = Boolean(String(voiceSettings.tts_base_url || "").trim() && String(voiceSettings.tts_api_key || "").trim());
  const notificationLabel = notificationsEnabled
    ? notificationDeviceClass() === "mobile"
      ? pushNotificationsEnabled
        ? "Notifications on (push)"
        : "Notifications pending"
      : notificationPermission === "granted" || notificationPermission === "unsupported"
        ? "Notifications on"
        : "Notifications pending"
    : "Notifications off";
  const announcementLabel = announcementEnabled ? "Announcements on" : "Announcements off";

  const renderRailActions = () => (
    <div className="sidebarBannerActions">
      <Button
        type="button"
        variant="ghost"
        className="brandMark"
        onClick={() => {
          setSidebarOpen(false);
          if (announcementEnabled) {
            void startAnnouncementPlayback(voiceSettings, { resetSource: true, force: true });
          }
        }}
      >
        Codoxear
      </Button>
      <div className="sidebarActionButtons">
        <Button
          type="button"
          variant="outline"
          size="icon"
          className={`iconAction legacyToggleAction${notificationsEnabled ? " isActive" : ""}`}
          aria-label={notificationLabel}
          title={notificationLabel}
          onClick={() => {
            void toggleNotifications();
          }}
        >
          <BellIcon />
          <span className="visuallyHidden">{notificationLabel}</span>
        </Button>
        <Button
          type="button"
          variant="outline"
          size="icon"
          className={`iconAction legacyToggleAction${announcementEnabled ? " isActive" : ""}`}
          aria-label={announcementLabel}
          title={announcementLabel}
          onClick={() => {
            void toggleAnnouncements();
            // Try to start playback from gesture if we are turning it on
            if (!announcementEnabled) {
              void startAnnouncementPlayback(voiceSettings, { resetSource: true, force: true });
            }
          }}
        >
          <VolumeIcon />
          <span className="visuallyHidden">{announcementLabel}</span>
        </Button>
      </div>
    </div>
  );

  const renderRailFooter = () => (
    <footer className="sidebarFooter">
      <Button type="button" variant="outline" className="footerAction"><span className="buttonGlyph">?</span><span>Help</span></Button>
      <Button type="button" variant="outline" className="footerAction" onClick={() => openVoiceSettings()}><span className="buttonGlyph">⚙</span><span>Settings</span></Button>
      <Button type="button" variant="outline" className="footerAction" onClick={() => { void logout(); }}><span className="buttonGlyph">→|</span><span>Log out</span></Button>
    </footer>
  );

  const renderSessionsRail = () => (
    <>
      <header className="sidebarBanner">{renderRailActions()}</header>
      <SessionsPane onNewSession={() => setNewSessionOpen(true)} />
      {renderRailFooter()}
    </>
  );

  const renderWorkspaceDetails = () => (
    sessionUiMatchesActiveSession ? <SessionWorkspace mode="details" /> : <EmptyDetailsWorkspace />
  );

  const openWorkspace = () => {
    if (shouldUseMobileWorkspaceSheet()) {
      setDetailsOpen(true);
      return;
    }
    setWorkspaceOpen(true);
  };

  const openMobileFiles = () => {
    setMobileToolsOpen(false);
    openFileViewer();
  };

  const openMobileWorkspace = () => {
    setMobileToolsOpen(false);
    openWorkspace();
  };

  const openMobileHarness = () => {
    setMobileToolsOpen(false);
    setHarnessOpen(true);
  };

  const playReplyBeep = () => {
    try {
      const AudioContextCtor = (window as any).AudioContext || (window as any).webkitAudioContext;
      if (!AudioContextCtor) {
        return;
      }
      const ctx = new AudioContextCtor();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.type = "triangle";
      osc.frequency.setValueAtTime(987.77, ctx.currentTime);
      gain.gain.setValueAtTime(0.0001, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.08, ctx.currentTime + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.18);
      osc.start(ctx.currentTime);
      osc.stop(ctx.currentTime + 0.18);
      osc.onended = () => {
        void ctx.close().catch(() => undefined);
      };
    } catch {
      // Best-effort local cue only.
    }
  };

  const finalResponseEventKey = (row: Record<string, unknown>) => {
    const messageId = typeof row.message_id === "string" ? row.message_id.trim() : "";
    if (messageId) return `id:${messageId}`;
    const ts = typeof row.ts === "number" ? row.ts : 0;
    const text = typeof row.notification_text === "string"
      ? row.notification_text
      : typeof row.text === "string"
        ? row.text
        : "";
    const normalizedText = text.replace(/\s+/g, " ").trim();
    return normalizedText ? `text:${ts}:${normalizedText}` : "";
  };

  const showDesktopNotification = (title: string, body: string, messageId?: string) => {
    if (notificationDeviceClass() !== "desktop" || notificationPermission !== "granted" || typeof Notification === "undefined") {
      return;
    }
    const trimmedBody = body.replace(/\s+/g, " ").trim();
    if (!trimmedBody) return;
    const id = String(messageId || "").trim();
    if (id && deliveredNotificationIdsRef.current.has(id)) return;
    new Notification(title.trim() || "Session", {
      body: trimmedBody.length <= 180 ? trimmedBody : `${trimmedBody.slice(0, 179).trimEnd()}...`,
      tag: id || `desktop:${Date.now()}`,
    });
    if (id) deliveredNotificationIdsRef.current.add(id);
  };

  useEffect(() => {
    if (
      notificationDeviceClass() !== "desktop"
      || !notificationsEnabled
      || notificationPermission !== "granted"
      || !activeSessionId
    ) {
      return;
    }

    const events = Array.isArray(bySessionId[activeSessionId]) ? bySessionId[activeSessionId] : [];
    for (const event of events) {
      if (!event || typeof event !== "object") continue;
      const row = event as Record<string, unknown>;
      if (row.role !== "assistant") continue;
      if (row.pending === true) continue;
      if (row.message_class !== "final_response") continue;
      const messageId = typeof row.message_id === "string" ? row.message_id : "";
      const notificationText = typeof row.notification_text === "string"
        ? row.notification_text
        : typeof row.text === "string"
          ? row.text
          : "";
      if (notificationText) {
        showDesktopNotification(activeTitle, notificationText, messageId);
        continue;
      }
      if (!messageId) {
        continue;
      }
      const lookupState = notificationLookupStateRef.current.get(messageId);
      if (lookupState?.terminal || (lookupState && lookupState.retryAfter > Date.now()) || resolvingNotificationIdsRef.current.has(messageId)) {
        continue;
      }
      resolvingNotificationIdsRef.current.add(messageId);
      api.getNotificationMessage(messageId)
        .then((response) => {
          const text = String(response.notification_text || "").trim();
          const status = String(response.summary_status || "").trim();
          if (text && (!status || FINAL_NOTIFICATION_SUMMARY_STATUSES.has(status))) {
            notificationLookupStateRef.current.set(messageId, {
              retryAfter: Number.POSITIVE_INFINITY,
              terminal: true,
            });
            showDesktopNotification(activeTitle, text, messageId);
            return;
          }

          notificationLookupStateRef.current.set(messageId, {
            retryAfter: status && FINAL_NOTIFICATION_SUMMARY_STATUSES.has(status)
              ? Number.POSITIVE_INFINITY
              : Date.now() + NOTIFICATION_MESSAGE_RETRY_MS,
            terminal: Boolean(status && FINAL_NOTIFICATION_SUMMARY_STATUSES.has(status)),
          });
        })
        .catch(() => undefined)
        .finally(() => {
          resolvingNotificationIdsRef.current.delete(messageId);
        });
    }
  }, [activeSessionId, activeTitle, bySessionId, notificationPermission, notificationsEnabled]);

  const openVoiceSettings = (status = "") => {
    setVoiceSettingsStatus(status);
    setVoiceBaseUrlDraft(String(voiceSettings.tts_base_url || ""));
    setVoiceApiKeyDraft(String(voiceSettings.tts_api_key || ""));
    setNarrationEnabledDraft(!!voiceSettings.tts_enabled_for_narration);
    setEnterToSendDraft(readLocalToggle("codoxear.enterToSend"));
    setVoiceSettingsOpen(true);
  };

  const closeVoiceSettings = () => {
    setVoiceSettingsOpen(false);
    setVoiceSettingsStatus("");
  };

  const toggleAnnouncements = async () => {
    const next = !announcementEnabled;
    if (next && !hasAnnouncementCredentials) {
      openVoiceSettings("Set the OpenAI-compatible API base URL and API key before enabling announcements.");
      return;
    }
    setAnnouncementEnabled(next);
  };

  const toggleNotifications = async () => {
    const next = !notificationsEnabled;
    if (!next) {
      if (notificationDeviceClass() === "mobile" && notificationEndpointRef.current) {
        const snapshot = await api.toggleNotificationSubscription(notificationEndpointRef.current, false);
        await syncNotificationSubscriptionState(snapshot);
      }
      setNotificationsEnabled(false);
      return;
    }
    if (typeof Notification !== "undefined" && Notification.permission !== "granted") {
      const permission = await Notification.requestPermission();
      setNotificationPermission(permission);
      if (permission !== "granted") {
        setNotificationsEnabled(false);
        return;
      }
    }
    if (typeof Notification !== "undefined" && Notification.permission === "granted") {
      setNotificationPermission("granted");
    }
    if (notificationDeviceClass() === "mobile") {
      if (typeof PushManager === "undefined") {
        setNotificationsEnabled(false);
        return;
      }
      const registration = await ensureVoiceServiceWorker();
      let subscription = await registration.pushManager.getSubscription();
      const publicKey = String(voiceSettings.notifications?.vapid_public_key || "").trim();
      if (!subscription) {
        if (!publicKey) {
          setNotificationsEnabled(false);
          return;
        }
        subscription = await registration.pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: base64UrlToUint8Array(publicKey),
        });
      }
      const snapshot = await api.upsertNotificationSubscription({
        subscription: subscription.toJSON(),
        user_agent: navigator.userAgent,
        device_label: "current-device",
        device_class: notificationDeviceClass(),
      });
      const endpoint = typeof subscription.endpoint === "string" ? subscription.endpoint : "";
      await syncNotificationSubscriptionState(snapshot, endpoint);
    }
    setNotificationsEnabled(true);
  };

  const saveVoiceSettings = async () => {
    setVoiceSettingsStatus("Saving...");
    try {
      const payload = {
        tts_enabled_for_narration: narrationEnabledDraft,
        tts_enabled_for_final_response: true,
        tts_base_url: voiceBaseUrlDraft.trim(),
        tts_api_key: voiceApiKeyDraft.trim(),
      };
      const response = await api.saveVoiceSettings(payload);
      const nextSettings = mergeVoiceSettings(response);
      setVoiceSettings(nextSettings);
      writeLocalToggle("codoxear.enterToSend", enterToSendDraft);
      setVoiceSettingsStatus("");
      setVoiceSettingsOpen(false);
    } catch (error) {
      setVoiceSettingsStatus(error instanceof Error ? `save error: ${error.message}` : "save error: unknown error");
    }
  };

  const playTestSound = () => {
    try {
      const AudioContext = (window as any).AudioContext || (window as any).webkitAudioContext;
      if (!AudioContext) {
        setVoiceSettingsStatus("Audio Context not supported in this browser.");
        return;
      }
      const ctx = new AudioContext();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.type = "sine";
      osc.frequency.setValueAtTime(440, ctx.currentTime);
      gain.gain.setValueAtTime(0, ctx.currentTime);
      gain.gain.linearRampToValueAtTime(0.1, ctx.currentTime + 0.1);
      gain.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + 0.8);
      osc.start(ctx.currentTime);
      osc.stop(ctx.currentTime + 0.8);
      setVoiceSettingsStatus("Playing test sound (beep)...");
      window.setTimeout(() => setVoiceSettingsStatus(""), 1500);
    } catch (error) {
      setVoiceSettingsStatus(error instanceof Error ? `test error: ${error.message}` : "test error: unknown error");
    }
  };

  const triggerTestAnnouncement = async () => {
    setVoiceSettingsStatus("Requesting test announcement...");
    try {
      await api.triggerTestAnnouncement();
      window.setTimeout(() => {
        api.getVoiceSettings()
          .then((response) => {
            const nextSettings = mergeVoiceSettings(response);
            setVoiceSettings(nextSettings);
            const lastError = String(nextSettings.audio?.last_error || "").trim();
            if (lastError) {
              setVoiceSettingsStatus(`Test announcement failed: ${lastError}`);
              return;
            }
            setVoiceSettingsStatus("Test announcement queued. If you still hear nothing, send me the Console [Audio] logs and the audio status below.");
          })
          .catch(() => {
            setVoiceSettingsStatus("Test announcement queued. If you still hear nothing, send me the Console [Audio] logs and the audio status below.");
          });
      }, 1600);
    } catch (error) {
      setVoiceSettingsStatus(error instanceof Error ? `test announcement error: ${error.message}` : "test announcement error: unknown error");
    }
  };

  return (
    <>
      <div className={shellClassName} data-testid="app-shell">
        <audio ref={liveAudioRef} className="liveAudioElement" preload="none" />
        <aside className="sidebarColumn desktopSessionsRail">{renderSessionsRail()}</aside>
        <section className="conversationColumn">
          <div className="conversationToolbar">
            <div className="conversationToolbarGroup conversationToolbarGroupPrimary">
              <Button type="button" variant="outline" size="sm" className="toolbarButton mobileSheetTrigger" onClick={() => setSidebarOpen(true)}>
                Sessions
              </Button>
              <div className="conversationTitle">{activeSessionId ? activeTitle : "No session selected"}</div>
            </div>
            <div className="conversationToolbarGroup conversationToolbarGroupActions">
              <Button type="button" variant="outline" size="sm" className="toolbarButton toolbarTextButton desktopToolbarButton" disabled={!activeSessionId} onClick={() => openFileViewer()}>Files</Button>
              <Button type="button" variant="outline" size="sm" className="toolbarButton toolbarTextButton desktopToolbarButton" disabled={!activeSessionId} onClick={openWorkspace}>Workspace</Button>
              <Button type="button" variant="outline" size="sm" className="toolbarButton toolbarTextButton desktopToolbarButton" disabled={!activeSessionId} onClick={() => setHarnessOpen(true)}>Harness</Button>
              <Button type="button" variant="outline" size="sm" className="toolbarButton mobileToolsTrigger" disabled={!activeSessionId} onClick={() => setMobileToolsOpen(true)}>
                <MoreIcon />
                <span>Tools</span>
              </Button>
            </div>
          </div>
          <ConversationPane onOpenFilePath={(path, line) => openFileViewer(path, line ?? null, "file")} />
          <Composer />
        </section>
        <div data-testid="mobile-sessions-sheet">
          <Sheet open={sidebarOpen}>
            <button type="button" className="sheetBackdropButton" aria-label="Close sessions panel" onClick={() => setSidebarOpen(false)} />
            <SheetContent side="left" className="mobileSheetContent" titleId="mobile-sessions-title">
              <div className="mobileSheetRail">
                <header className="mobileSheetHeader">
                  <h2 id="mobile-sessions-title">Sessions</h2>
                  <Button type="button" variant="ghost" size="sm" onClick={() => setSidebarOpen(false)}>Close</Button>
                </header>
                {renderSessionsRail()}
              </div>
            </SheetContent>
          </Sheet>
        </div>
        <div data-testid="mobile-workspace-sheet">
          <Sheet open={detailsOpen}>
            <button type="button" className="sheetBackdropButton" aria-label="Close workspace panel" onClick={() => setDetailsOpen(false)} />
            <SheetContent side="right" className="mobileSheetContent" titleId="mobile-workspace-title">
              <div className="mobileWorkspaceSheet">
                <header className="mobileSheetHeader">
                  <h2 id="mobile-workspace-title">Workspace</h2>
                  <Button type="button" variant="ghost" size="sm" onClick={() => setDetailsOpen(false)}>Close</Button>
                </header>
                {renderWorkspaceDetails()}
              </div>
            </SheetContent>
          </Sheet>
        </div>
        <div data-testid="mobile-tools-sheet">
          <Sheet open={mobileToolsOpen}>
            <button type="button" className="sheetBackdropButton" aria-label="Close tools panel" onClick={() => setMobileToolsOpen(false)} />
            <SheetContent side="right" className="mobileSheetContent mobileToolsContent" titleId="mobile-tools-title">
              <div className="mobileToolsSheet">
                <header className="mobileSheetHeader">
                  <h2 id="mobile-tools-title">Tools</h2>
                  <Button type="button" variant="ghost" size="sm" onClick={() => setMobileToolsOpen(false)}>Close</Button>
                </header>
                <div className="mobileToolsActions">
                  <Button type="button" variant="outline" className="mobileToolsAction" disabled={!activeSessionId} onClick={openMobileFiles}>Files</Button>
                  <Button type="button" variant="outline" className="mobileToolsAction" disabled={!activeSessionId} onClick={openMobileWorkspace}>Workspace</Button>
                  <Button type="button" variant="outline" className="mobileToolsAction" disabled={!activeSessionId} onClick={openMobileHarness}>Harness</Button>
                </div>
              </div>
            </SheetContent>
          </Sheet>
        </div>
      </div>
      <Dialog open={workspaceOpen} onOpenChange={setWorkspaceOpen}>
        <DialogContent className="workspaceDialog max-w-none" titleId="workspace-dialog-title">
          <div data-testid="workspace-dialog" className="workspaceDialogBody">
            <DialogHeader className="workspaceDialogHeader">
              <div className="flex items-center justify-between gap-3">
                <div className="space-y-1">
                  <DialogTitle id="workspace-dialog-title">Workspace</DialogTitle>
                  <p className="text-sm text-muted-foreground">Inspect session details, queue state, tracked files, and UI requests.</p>
                </div>
                <Button type="button" variant="ghost" size="sm" onClick={() => setWorkspaceOpen(false)}>Close</Button>
              </div>
            </DialogHeader>
            <div className="min-h-0 flex-1 overflow-y-auto p-5">
              {renderWorkspaceDetails()}
            </div>
          </div>
        </DialogContent>
      </Dialog>
      {voiceSettingsOpen ? (
        <div className="dialogBackdrop" onClick={closeVoiceSettings}>
          <section className="dialogCard legacyDialog voiceSettingsDialog" onClick={(event) => event.stopPropagation()}>
            <header className="dialogHeader">
              <h2>Settings</h2>
              <p>Configure announcements and notification delivery.</p>
            </header>
            <div className="newSessionForm">
              {voiceSettingsStatus ? <p className="fieldHint voiceSettingsStatus">{voiceSettingsStatus}</p> : null}
              <label className="fieldBlock">
                <span className="fieldLabel">OpenAI-compatible API base URL</span>
                <input
                  value={voiceBaseUrlDraft}
                  onInput={(event) => setVoiceBaseUrlDraft(event.currentTarget.value)}
                  onChange={(event) => setVoiceBaseUrlDraft(event.currentTarget.value)}
                  placeholder="https://api.openai.com/v1"
                />
              </label>
              <label className="fieldBlock">
                <span className="fieldLabel">OpenAI-compatible API key</span>
                <input
                  value={voiceApiKeyDraft}
                  onInput={(event) => setVoiceApiKeyDraft(event.currentTarget.value)}
                  onChange={(event) => setVoiceApiKeyDraft(event.currentTarget.value)}
                  placeholder="sk-..."
                  type="password"
                />
              </label>
              <div className="fieldBlock toggleField">
                <span className="fieldLabel">Announcements</span>
                <label className="checkField">
                  <input
                    type="checkbox"
                    checked={narrationEnabledDraft}
                    onChange={(event) => setNarrationEnabledDraft(event.currentTarget.checked)}
                  />
                  <span>Announce narration messages</span>
                </label>
              </div>
              <div className="fieldBlock toggleField">
                <span className="fieldLabel">Composer</span>
                <label className="checkField">
                  <input
                    type="checkbox"
                    checked={enterToSendDraft}
                    onChange={(event) => setEnterToSendDraft(event.currentTarget.checked)}
                  />
                  <span>Press Enter to send</span>
                </label>
              </div>
              <div className="fieldBlock toggleField">
                <span className="fieldLabel">Reply sound</span>
                <label className="checkField">
                  <input
                    type="checkbox"
                    checked={replySoundEnabled}
                    onChange={(event) => setReplySoundEnabled(event.currentTarget.checked)}
                  />
                  <span>Play a short beep when the assistant finishes a reply</span>
                </label>
              </div>
              <div className="voiceSettingsMeta fieldHint">
                <span>Listeners: {voiceSettings.audio?.active_listener_count ?? 0}</span>
                <span>Queue: {voiceSettings.audio?.queue_depth ?? 0}</span>
                <span>Segments: {voiceSettings.audio?.segment_count ?? 0}</span>
                <span>Mobile notifications: {voiceSettings.notifications?.enabled_devices ?? 0}/{voiceSettings.notifications?.total_devices ?? 0}</span>
              </div>
              {voiceSettings.audio?.last_error ? (
                <p className="fieldHint voiceSettingsStatus">Audio error: {voiceSettings.audio.last_error}</p>
              ) : null}
              <div className="formActions dialogFormActions">
                <button type="button" className="secondaryButton" onClick={playTestSound}>Test Sound</button>
                <button type="button" className="secondaryButton" onClick={() => { void triggerTestAnnouncement(); }}>Test Announcement</button>
                <div className="flex-1" />
                <button type="button" className="secondaryButton" onClick={closeVoiceSettings}>Cancel</button>
                <button type="button" className="primaryButton" onClick={() => { void saveVoiceSettings(); }}>Save</button>
              </div>
            </div>
          </section>
        </div>
      ) : null}
      <FileViewerDialog
        open={fileViewerOpen}
        sessionId={activeSessionId}
        files={sessionUiMatchesActiveSession ? files : []}
        initialPath={fileViewerPath}
        initialLine={fileViewerLine}
        initialMode={fileViewerMode}
        openRequestKey={fileViewerRequestKey}
        onClose={closeFileViewer}
      />
      <HarnessDialog open={harnessOpen} sessionId={activeSessionId} onClose={() => setHarnessOpen(false)} />
      <NewSessionDialog open={newSessionOpen} onClose={() => setNewSessionOpen(false)} />
    </>
  );
}
