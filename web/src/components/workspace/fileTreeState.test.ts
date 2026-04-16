import { describe, expect, it } from "vitest";

import {
  createTreeStateFromEntries,
  mergeDirectoryEntries,
  setNodeError,
  setNodeExpanded,
  setNodeLoading,
} from "./fileTreeState";

const rootEntries = [
  { name: "src", path: "src", kind: "dir" as const },
  { name: "README.md", path: "README.md", kind: "file" as const },
];

describe("fileTreeState", () => {
  it("creates sorted root paths and node records", () => {
    const state = createTreeStateFromEntries(rootEntries);

    expect(state.rootPaths).toEqual(["src", "README.md"]);
    expect(state.nodesByPath.src).toMatchObject({
      path: "src",
      kind: "dir",
      expanded: false,
      loaded: false,
      loading: false,
      childPaths: [],
    });
  });

  it("merges directory children without replacing unrelated nodes", () => {
    const initial = createTreeStateFromEntries(rootEntries);
    const untouchedReadme = initial.nodesByPath["README.md"];

    const merged = mergeDirectoryEntries(initial, "src", [
      { name: "main.tsx", path: "src/main.tsx", kind: "file" },
      { name: "components", path: "src/components", kind: "dir" },
    ]);

    expect(merged.nodesByPath.src.childPaths).toEqual(["src/components", "src/main.tsx"]);
    expect(merged.nodesByPath.src.loaded).toBe(true);
    expect(merged.nodesByPath["README.md"]).toBe(untouchedReadme);
  });

  it("toggles only the targeted node", () => {
    const initial = createTreeStateFromEntries(rootEntries);
    const untouchedReadme = initial.nodesByPath["README.md"];

    const expanded = setNodeExpanded(initial, "src", true);

    expect(expanded.nodesByPath.src.expanded).toBe(true);
    expect(expanded.nodesByPath["README.md"]).toBe(untouchedReadme);
  });

  it("stores node-local loading and error state", () => {
    const initial = createTreeStateFromEntries(rootEntries);
    const loadingState = setNodeLoading(initial, "src", true);
    const errorState = setNodeError(loadingState, "src", "Unable to list files");

    expect(loadingState.nodesByPath.src.loading).toBe(true);
    expect(errorState.nodesByPath.src).toMatchObject({
      loading: false,
      error: "Unable to list files",
    });
  });

  it("keeps sibling branch records referentially stable when another node expands", () => {
    const initial = mergeDirectoryEntries(
      createTreeStateFromEntries([
        { name: "docs", path: "docs", kind: "dir" },
        { name: "src", path: "src", kind: "dir" },
      ]),
      "docs",
      [{ name: "intro.md", path: "docs/intro.md", kind: "file" }],
    );

    const srcBefore = initial.nodesByPath.src;
    const docsChildBefore = initial.nodesByPath["docs/intro.md"];

    const expanded = setNodeExpanded(initial, "src", true);

    expect(expanded.nodesByPath.src).not.toBe(srcBefore);
    expect(expanded.nodesByPath["docs/intro.md"]).toBe(docsChildBefore);
  });
});
