import { describe, expect, it } from "vitest";

import { deriveWorkspaceViewData } from "./sessionWorkspaceViewModel";

describe("deriveWorkspaceViewData", () => {
  it("partitions priority diagnostics, generic diagnostics, queue items, and todo snapshot", () => {
    const data = deriveWorkspaceViewData({
      diagnostics: {
        log_path: "/tmp/pi-broker.log",
        session_file_path: "/tmp/pi-session.jsonl",
        updated_ts: 1_700_000_100,
        queue_len: 2,
        status: "ok",
        todo_snapshot: {
          available: true,
          error: false,
          progress_text: "1/2 completed",
          items: [{ id: 1, title: "Explore context", status: "completed" }],
        },
      },
      queue: { items: [{ text: "next task" }, "after that"] },
    });

    expect(data.diagnosticsEntries).toHaveLength(6);
    expect(data.priorityDetailEntries.map(([key]: [string, unknown]) => key)).toEqual(["log_path", "session_file_path", "updated_ts"]);
    expect(data.genericDetailEntries.map(([key]: [string, unknown]) => key)).toEqual(["queue_len", "status"]);
    expect(data.queueItems).toEqual(["next task", "after that"]);
    expect(data.todoSnapshot).toMatchObject({
      available: true,
      progress_text: "1/2 completed",
      items: [{ title: "Explore context", status: "completed" }],
    });
  });

  it("returns empty stable sections for missing diagnostics and queue", () => {
    const data = deriveWorkspaceViewData({ diagnostics: null, queue: null });

    expect(data.diagnosticsEntries).toEqual([]);
    expect(data.detailEntries).toEqual([]);
    expect(data.priorityDetailEntries).toEqual([]);
    expect(data.genericDetailEntries).toEqual([]);
    expect(data.queueItems).toEqual([]);
    expect(data.todoSnapshot).toEqual({ available: false, error: false, items: [] });
  });
});
