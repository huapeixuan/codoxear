import { api } from "../../lib/api";

export interface ComposerState {
  draft: string;
  sending: boolean;
}

export interface ComposerStore {
  getState(): ComposerState;
  subscribe(listener: () => void): () => void;
  setDraft(value: string): void;
  submit(sessionId: string): Promise<void>;
}

export function createComposerStore(): ComposerStore {
  let state: ComposerState = { draft: "", sending: false };
  const listeners = new Set<() => void>();

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
      
      state = { ...state, sending: true };
      emit();

      try {
        await api.sendMessage(sessionId, state.draft);
        state = { draft: "", sending: false };
        emit();
      } catch (error) {
        state = { ...state, sending: false };
        emit();
        throw error;
      }
    },
  };
}
