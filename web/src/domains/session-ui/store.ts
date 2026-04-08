import { api } from "../../lib/api";
import type { SessionUiRequest } from "../../lib/types";

export interface SessionUiState {
  sessionId: string | null;
  requests: SessionUiRequest[];
  diagnostics: Record<string, unknown> | null;
  queue: Record<string, unknown> | null;
  files: string[];
  loading: boolean;
}

export interface SessionUiRefreshOptions {
  agentBackend?: string;
}

export interface SessionUiStore {
  getState(): SessionUiState;
  subscribe(listener: () => void): () => void;
  refresh(sessionId: string, options?: SessionUiRefreshOptions): Promise<void>;
}

export function createSessionUiStore(): SessionUiStore {
  let state: SessionUiState = {
    sessionId: null,
    requests: [],
    diagnostics: null,
    queue: null,
    files: [],
    loading: false,
  };
  const listeners = new Set<() => void>();
  let currentRefreshId = 0;

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
    async refresh(sessionId: string, options?: SessionUiRefreshOptions) {
      const refreshId = ++currentRefreshId;
      const preserveCurrentState = state.sessionId === sessionId;
      state = {
        sessionId,
        requests: preserveCurrentState ? state.requests : [],
        diagnostics: preserveCurrentState ? state.diagnostics : null,
        queue: preserveCurrentState ? state.queue : null,
        files: preserveCurrentState ? state.files : [],
        loading: true,
      };
      emit();

      try {
        const requestsPromise = options?.agentBackend === "pi" ? api.getSessionUiState(sessionId) : Promise.resolve({ requests: [] });
        const [uiState, diagnostics, queue, files] = await Promise.all([
          requestsPromise,
          api.getDiagnostics(sessionId),
          api.getQueue(sessionId),
          api.getFiles(sessionId),
        ]);
        if (refreshId !== currentRefreshId) {
          return;
        }

        state = {
          sessionId,
          requests: uiState.requests,
          diagnostics: diagnostics as Record<string, unknown>,
          queue: queue as Record<string, unknown>,
          files: files.files,
          loading: false,
        };
        emit();
      } catch (error) {
        if (refreshId === currentRefreshId) {
          state = { ...state, loading: false };
          emit();
          throw error;
        }
      }
    },
  };
}
