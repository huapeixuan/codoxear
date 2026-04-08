import { createContext } from "preact";
import { useContext } from "preact/hooks";
import { useSyncExternalStore } from "preact/compat";
import { createMessagesStore, type MessagesStore } from "../domains/messages/store";
import { createSessionsStore, type SessionsStore } from "../domains/sessions/store";
import { createComposerStore, type ComposerStore } from "../domains/composer/store";
import { createSessionUiStore, type SessionUiStore } from "../domains/session-ui/store";

const defaultSessionsStore = createSessionsStore();
const defaultMessagesStore = createMessagesStore();
const defaultComposerStore = createComposerStore();
const defaultSessionUiStore = createSessionUiStore();

const SessionsStoreContext = createContext<SessionsStore>(defaultSessionsStore);
const MessagesStoreContext = createContext<MessagesStore>(defaultMessagesStore);
const ComposerStoreContext = createContext<ComposerStore>(defaultComposerStore);
const SessionUiStoreContext = createContext<SessionUiStore>(defaultSessionUiStore);

interface AppProvidersProps {
  children: preact.ComponentChildren;
  sessionsStore?: SessionsStore;
  messagesStore?: MessagesStore;
  composerStore?: ComposerStore;
  sessionUiStore?: SessionUiStore;
}

export function AppProviders({
  children,
  sessionsStore = defaultSessionsStore,
  messagesStore = defaultMessagesStore,
  composerStore = defaultComposerStore,
  sessionUiStore = defaultSessionUiStore,
}: AppProvidersProps) {
  return (
    <SessionsStoreContext.Provider value={sessionsStore}>
      <MessagesStoreContext.Provider value={messagesStore}>
        <ComposerStoreContext.Provider value={composerStore}>
          <SessionUiStoreContext.Provider value={sessionUiStore}>{children}</SessionUiStoreContext.Provider>
        </ComposerStoreContext.Provider>
      </MessagesStoreContext.Provider>
    </SessionsStoreContext.Provider>
  );
}

export function useSessionsStore() {
  const store = useContext(SessionsStoreContext);
  return useSyncExternalStore(store.subscribe, store.getState);
}

export function useMessagesStore() {
  const store = useContext(MessagesStoreContext);
  return useSyncExternalStore(store.subscribe, store.getState);
}

export function useComposerStore() {
  const store = useContext(ComposerStoreContext);
  return useSyncExternalStore(store.subscribe, store.getState);
}

export function useSessionUiStore() {
  const store = useContext(SessionUiStoreContext);
  return useSyncExternalStore(store.subscribe, store.getState);
}

export function useSessionsStoreApi() {
  return useContext(SessionsStoreContext);
}

export function useMessagesStoreApi() {
  return useContext(MessagesStoreContext);
}

export function useComposerStoreApi() {
  return useContext(ComposerStoreContext);
}

export function useSessionUiStoreApi() {
  return useContext(SessionUiStoreContext);
}
