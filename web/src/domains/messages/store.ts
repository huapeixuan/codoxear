import { api } from "../../lib/api";
import type { MessageEvent } from "../../lib/types";

export interface MessagesState {
  bySessionId: Record<string, MessageEvent[]>;
  offsetsBySessionId: Record<string, number>;
  loading: boolean;
}

export interface MessagesStore {
  getState(): MessagesState;
  subscribe(listener: () => void): () => void;
  loadInitial(sessionId: string): Promise<void>;
  poll(sessionId: string): Promise<void>;
}

export function createMessagesStore(): MessagesStore {
  let state: MessagesState = {
    bySessionId: {},
    offsetsBySessionId: {},
    loading: false,
  };
  const listeners = new Set<() => void>();
  const currentLoadIds: Record<string, number> = {};

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
  };
}
