import type { TodoSnapshot, TodoSnapshotItem } from "../../lib/types";

const PRIORITIZED_DETAIL_KEYS = new Set(["session_file_path", "log_path", "updated_ts"]);

function entriesFromRecord(value: Record<string, unknown> | null) {
  return value ? Object.entries(value) : [];
}

function queueItemsFromValue(queue: Record<string, unknown> | null) {
  const rawItems = queue?.items;
  if (!Array.isArray(rawItems)) {
    return [];
  }
  return rawItems.map((item) => {
    if (item && typeof item === "object" && "text" in item) {
      return String((item as { text?: unknown }).text ?? "");
    }
    return String(item);
  });
}

function normalizeTodoItem(value: unknown): TodoSnapshotItem | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const item = value as Record<string, unknown>;
  return {
    id: typeof item.id === "number" || typeof item.id === "string" ? item.id : undefined,
    title: typeof item.title === "string" ? item.title : undefined,
    status: typeof item.status === "string" ? item.status : undefined,
    description: typeof item.description === "string" ? item.description : undefined,
  };
}

function normalizeTodoSnapshot(snapshot: unknown): TodoSnapshot {
  if (!snapshot || typeof snapshot !== "object") {
    return { available: false, error: false, items: [] };
  }
  const raw = snapshot as Record<string, unknown>;
  return {
    available: raw.available === true,
    error: raw.error === true,
    progress_text: typeof raw.progress_text === "string" ? raw.progress_text : undefined,
    items: Array.isArray(raw.items)
      ? raw.items.map(normalizeTodoItem).filter((item): item is TodoSnapshotItem => Boolean(item))
      : [],
  };
}

export function deriveWorkspaceViewData({
  diagnostics,
  queue,
}: {
  diagnostics: Record<string, unknown> | null;
  queue: Record<string, unknown> | null;
}) {
  const diagnosticsEntries = entriesFromRecord(diagnostics);
  const todoSnapshot = normalizeTodoSnapshot(diagnostics && typeof diagnostics === "object" ? (diagnostics as { todo_snapshot?: unknown }).todo_snapshot : null);
  const detailEntries = diagnosticsEntries.filter(([key]) => key !== "todo_snapshot");
  const priorityDetailEntries = detailEntries.filter(([key]) => PRIORITIZED_DETAIL_KEYS.has(key));
  const genericDetailEntries = detailEntries.filter(([key]) => !PRIORITIZED_DETAIL_KEYS.has(key));
  const queueItems = queueItemsFromValue(queue);

  return {
    diagnosticsEntries,
    todoSnapshot,
    detailEntries,
    priorityDetailEntries,
    genericDetailEntries,
    queueItems,
  };
}
