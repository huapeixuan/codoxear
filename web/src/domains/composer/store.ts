import { api } from "../../lib/api";

export interface PendingComposerMessage {
  localId: string;
  role: "user";
  text: string;
  pending: true;
  [key: string]: unknown;
}

export interface ComposerState {
  draft: string;
  sending: boolean;
  pendingBySessionId: Record<string, PendingComposerMessage[]>;
}

export interface ComposerStore {
  getState(): ComposerState;
  subscribe(listener: () => void): () => void;
  setDraft(value: string): void;
  submit(sessionId: string): Promise<void>;
}

export function createComposerStore(): ComposerStore {
  let state: ComposerState = { draft: "", sending: false, pendingBySessionId: {} };
  const listeners = new Set<() => void>();
  let nextPendingId = 0;

  const emit = () => {
    for (const listener of listeners) {
      listener();
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
    setDraft(value: string) {
      state = { ...state, draft: value };
      emit();
    },
    async submit(sessionId: string) {
      if (!state.draft.trim() || state.sending) return;

      nextPendingId += 1;
      const text = state.draft;
      const pendingMessage: PendingComposerMessage = {
        localId: `local-pending-${nextPendingId}`,
        role: "user",
        text,
        pending: true,
      };

      state = {
        ...state,
        draft: "",
        sending: true,
        pendingBySessionId: {
          ...state.pendingBySessionId,
          [sessionId]: [...(state.pendingBySessionId[sessionId] ?? []), pendingMessage],
        },
      };
      emit();

      try {
        await api.sendMessage(sessionId, text);
        state = {
          ...state,
          sending: false,
          pendingBySessionId: {
            ...state.pendingBySessionId,
            [sessionId]: (state.pendingBySessionId[sessionId] ?? []).filter((item) => item.localId !== pendingMessage.localId),
          },
        };
        emit();
      } catch (error) {
        state = {
          ...state,
          draft: state.draft ? state.draft : text,
          sending: false,
          pendingBySessionId: {
            ...state.pendingBySessionId,
            [sessionId]: (state.pendingBySessionId[sessionId] ?? []).filter((item) => item.localId !== pendingMessage.localId),
          },
        };
        emit();
        throw error;
      }
    },
  };
}
