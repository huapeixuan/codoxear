import { api } from "../../lib/api";
import type { MessageEvent } from "../../lib/types";

export interface MessagesState {
  bySessionId: Record<string, MessageEvent[]>;
  offsetsBySessionId: Record<string, number>;
  hasOlderBySessionId: Record<string, boolean>;
  olderBeforeBySessionId: Record<string, number>;
  loadingOlderBySessionId: Record<string, boolean>;
  loading: boolean;
}

export interface MessagesStore {
  getState(): MessagesState;
  subscribe(listener: () => void): () => void;
  loadInitial(sessionId: string): Promise<void>;
  poll(sessionId: string): Promise<void>;
  loadOlder(sessionId: string, limit?: number): Promise<void>;
}

export function createMessagesStore(): MessagesStore {
  let state: MessagesState = {
    bySessionId: {},
    offsetsBySessionId: {},
    hasOlderBySessionId: {},
    olderBeforeBySessionId: {},
    loadingOlderBySessionId: {},
    loading: false,
  };
  const listeners = new Set<() => void>();
  const currentLoadIds: Record<string, number> = {};
  const currentOlderLoadIds: Record<string, number> = {};

  const emit = () => {
    for (const listener of listeners) {
      listener();
    }
  };

  const load = async (sessionId: string, init: boolean) => {
    const loadId = (currentLoadIds[sessionId] || 0) + 1;
    currentLoadIds[sessionId] = loadId;
    state = { ...state, loading: true };
    emit();

    try {
      const data = await api.listMessages(sessionId, init, undefined, init ? undefined : state.offsetsBySessionId[sessionId]);
      if (loadId !== currentLoadIds[sessionId]) {
        return;
      }
      const priorEvents = init ? [] : state.bySessionId[sessionId] ?? [];
      state = {
        bySessionId: {
          ...state.bySessionId,
          [sessionId]: init ? data.events : [...priorEvents, ...data.events],
        },
        offsetsBySessionId: {
          ...state.offsetsBySessionId,
          [sessionId]: typeof data.offset === "number" ? data.offset : state.offsetsBySessionId[sessionId] ?? 0,
        },
        hasOlderBySessionId: {
          ...state.hasOlderBySessionId,
          [sessionId]: init ? data.has_older === true : state.hasOlderBySessionId[sessionId] ?? false,
        },
        olderBeforeBySessionId: {
          ...state.olderBeforeBySessionId,
          [sessionId]: init
            ? typeof data.next_before === "number"
              ? data.next_before
              : 0
            : state.olderBeforeBySessionId[sessionId] ?? 0,
        },
        loadingOlderBySessionId: {
          ...state.loadingOlderBySessionId,
          [sessionId]: false,
        },
        loading: false,
      };
      emit();
    } catch (error) {
      if (loadId === currentLoadIds[sessionId]) {
        state = { ...state, loading: false };
        emit();
        throw error;
      }
    }
  };

  const loadOlder = async (sessionId: string, limit = 80) => {
    const before = state.olderBeforeBySessionId[sessionId] ?? 0;
    if (before <= 0 && !state.hasOlderBySessionId[sessionId]) {
      return;
    }

    const loadId = (currentOlderLoadIds[sessionId] || 0) + 1;
    currentOlderLoadIds[sessionId] = loadId;
    state = {
      ...state,
      loadingOlderBySessionId: {
        ...state.loadingOlderBySessionId,
        [sessionId]: true,
      },
    };
    emit();

    try {
      const data = await api.listMessages(sessionId, true, undefined, undefined, before, limit);
      if (loadId !== currentOlderLoadIds[sessionId]) {
        return;
      }

      state = {
        ...state,
        bySessionId: {
          ...state.bySessionId,
          [sessionId]: [...data.events, ...(state.bySessionId[sessionId] ?? [])],
        },
        offsetsBySessionId: {
          ...state.offsetsBySessionId,
          [sessionId]: typeof data.offset === "number" ? data.offset : state.offsetsBySessionId[sessionId] ?? 0,
        },
        hasOlderBySessionId: {
          ...state.hasOlderBySessionId,
          [sessionId]: data.has_older === true,
        },
        olderBeforeBySessionId: {
          ...state.olderBeforeBySessionId,
          [sessionId]: typeof data.next_before === "number" ? data.next_before : 0,
        },
        loadingOlderBySessionId: {
          ...state.loadingOlderBySessionId,
          [sessionId]: false,
        },
      };
      emit();
    } catch (error) {
      if (loadId === currentOlderLoadIds[sessionId]) {
        state = {
          ...state,
          loadingOlderBySessionId: {
            ...state.loadingOlderBySessionId,
            [sessionId]: false,
          },
        };
        emit();
        throw error;
      }
    }
  };

  return {
    getState: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    loadInitial(sessionId: string) {
      return load(sessionId, true);
    },
    poll(sessionId: string) {
      return load(sessionId, false);
    },
    loadOlder(sessionId: string, limit?: number) {
      return loadOlder(sessionId, limit);
    },
  };
}
