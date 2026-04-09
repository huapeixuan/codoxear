import { render } from "preact";
import { act } from "preact/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FileViewerDialog } from "./FileViewerDialog";
import { clearRememberedFileSelections } from "./fileSelectionState";

vi.mock("../../lib/api", () => ({
  api: {
    getFiles: vi.fn(),
    getFileRead: vi.fn(),
    getGitFileVersions: vi.fn(),
  },
}));

vi.mock("./MonacoWorkspace", () => ({
  MonacoWorkspace: (props: any) => (
    <div
      data-testid="monaco-workspace"
      data-line={props.line == null ? "" : String(props.line)}
      data-mode={props.mode}
      data-path={props.path}
    >
      {props.mode}:{props.path}
    </div>
  ),
}));

let root: HTMLDivElement | null = null;

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

async function settle(count = 4) {
  for (let index = 0; index < count; index += 1) {
    await flush();
  }
}

describe("FileViewerDialog", () => {
  afterEach(() => {
    clearRememberedFileSelections();
    vi.clearAllMocks();
    if (root) {
      render(null, root);
      root.remove();
      root = null;
    }
  });

  it("defaults to diff mode for the first tracked file and loads git file versions", async () => {
    const { api } = await import("../../lib/api");
    (api as any).getFiles.mockResolvedValue({ ok: true, files: ["src/main.tsx"] });
    (api as any).getGitFileVersions.mockResolvedValue({
      ok: true,
      path: "src/main.tsx",
      base_exists: true,
      current_exists: true,
      base_text: "const before = true;",
      current_text: "const after = true;",
    } as any);
    (api as any).getFileRead.mockResolvedValue({ ok: true, kind: "text", text: "const after = true;" });

    root = document.createElement("div");
    document.body.appendChild(root);
    await act(async () => {
      render(
        <FileViewerDialog open sessionId="sess-diff" files={["src/main.tsx"]} onClose={() => undefined} />,
        root!,
      );
      await settle(8);
    });

    expect((api as any).getFiles).toHaveBeenCalledWith("sess-diff", expect.any(AbortSignal));
    expect((api as any).getGitFileVersions).toHaveBeenCalledWith("sess-diff", "src/main.tsx", expect.any(AbortSignal));
    expect(root.textContent).toContain("Diff");
  });

  it("can switch from diff mode to file and markdown preview modes", async () => {
    const { api } = await import("../../lib/api");
    (api as any).getFiles.mockResolvedValue({ ok: true, files: ["docs/intro.md"] });
    (api as any).getGitFileVersions.mockResolvedValue({
      ok: true,
      path: "docs/intro.md",
      base_exists: true,
      current_exists: true,
      base_text: "# Before",
      current_text: "# After",
    } as any);
    (api as any).getFileRead.mockResolvedValue({ ok: true, kind: "text", text: "# Hello\n\nBody" });

    root = document.createElement("div");
    document.body.appendChild(root);
    await act(async () => {
      render(
        <FileViewerDialog open sessionId="sess-preview" files={["docs/intro.md"]} onClose={() => undefined} />,
        root!,
      );
      await settle(8);
    });

    const fileButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent === "File") as HTMLButtonElement | undefined;
    const previewButton = Array.from(root.querySelectorAll("button")).find((button) => button.textContent === "Preview") as HTMLButtonElement | undefined;
    expect(fileButton).toBeDefined();
    expect(previewButton).toBeDefined();

    act(() => {
      fileButton?.click();
    });
    await settle(6);
    expect(api.getFileRead).toHaveBeenCalledWith("sess-preview", "docs/intro.md", expect.any(AbortSignal));
    expect(root.textContent).toContain("docs/intro.md");

    act(() => {
      previewButton?.click();
    });
    await settle(6);
    expect(root.querySelector(".filePreview article h1")?.textContent).toBe("Hello");
    expect(root.querySelector(".filePreview article")?.textContent).toContain("Body");
  });

  it("opens explicit file references in file mode and preserves the requested line", async () => {
    const { api } = await import("../../lib/api");
    (api as any).getFiles.mockResolvedValue({ ok: true, files: ["src/main.tsx"] });
    (api as any).getFileRead.mockResolvedValue({ ok: true, kind: "text", text: "line 1\nline 2" });

    root = document.createElement("div");
    document.body.appendChild(root);
    await act(async () => {
      render(
        <FileViewerDialog
          open
          sessionId="sess-line"
          files={[]}
          initialPath="src/main.tsx"
          initialLine={18}
          onClose={() => undefined}
        />,
        root!,
      );
      await settle(8);
    });

    expect((api as any).getFileRead).toHaveBeenCalledWith("sess-line", "src/main.tsx", expect.any(AbortSignal));
    expect(root.textContent).toContain("line 18");
  });
});
