import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(process.cwd(), "src/styles/global.css"), "utf-8");

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function blockBody(source: string, openBraceIndex: number) {
  let depth = 1;

  for (let index = openBraceIndex + 1; index < source.length; index += 1) {
    const char = source[index];

    if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
    }

    if (depth === 0) {
      return source.slice(openBraceIndex + 1, index);
    }
  }

  throw new Error(`Unclosed CSS block starting at ${openBraceIndex}`);
}

function ruleBody(source: string, selector: string) {
  const match = new RegExp(`${escapeRegExp(selector)}\\s*\\{`, "m").exec(source);
  expect(match, `Expected CSS rule for ${selector}`).not.toBeNull();

  return blockBody(source, match!.index + match![0].length - 1);
}

function expectRuleToContain(source: string, selector: string, declaration: string) {
  const rulePattern = new RegExp(`${escapeRegExp(selector)}\\s*\\{`, "gm");
  const matches = [...source.matchAll(rulePattern)];
  expect(matches.length, `Expected CSS rule for ${selector}`).toBeGreaterThan(0);

  const hasDeclaration = matches.some((match) => blockBody(source, match.index! + match[0].length - 1).includes(declaration));
  expect(hasDeclaration, `Expected ${selector} to include ${declaration}`).toBe(true);
}

function mediaBody(source: string, query: string) {
  const match = new RegExp(`@media\\s*${escapeRegExp(query)}\\s*\\{`, "m").exec(source);
  expect(match, `Expected @media ${query} block`).not.toBeNull();

  return blockBody(source, match!.index + match![0].length - 1);
}

describe("conversation layout scroll guards", () => {
  it("keeps the conversation column as a shrinkable flex stack so the composer stays visible", () => {
    expect(css).toMatch(/\.appShell\.legacyShell\s*\{[^}]*grid-template-rows:\s*1fr;/);
    expect(css).toMatch(/\.appShell\.legacyShell\s*\{[^}]*position:\s*fixed;/);
    expect(css).toMatch(/\.appShell\.legacyShell\s*\{[^}]*inset:\s*0;/);
    expect(css).toMatch(/\.conversationColumn\s*\{[^}]*display:\s*flex;/);
    expect(css).toMatch(/\.conversationColumn\s*\{[^}]*flex-direction:\s*column;/);
    expect(css).toMatch(/\.conversationColumn\s*\{[^}]*max-height:\s*100dvh;/);
    expect(css).toMatch(/\.conversationColumn\s*\{[^}]*min-height:\s*0;/);
    expect(css).toMatch(/\.conversationPane\s*\{[^}]*min-height:\s*0;/);
    expect(css).toMatch(/\.conversationPane\s*\{[^}]*flex:\s*1 1 0;/);
  });

  it("collapses the third shell column when the workspace rail is hidden", () => {
    expect(css).toMatch(/\.appShell\.legacyShell\.noWorkspace\s*\{[^}]*grid-template-columns:\s*var\(--sidebar-w\)\s+minmax\(0,\s*1fr\);/);
  });

  it("keeps composer todo selectors in the global shell stylesheet", () => {
    expect(css).toMatch(/\.composerStack\s*\{/);
    expect(css).toMatch(/\.composerTodoBar\s*\{/);
    expect(css).toMatch(/\.composerTodoBarButton\.isExpanded\s*\{/);
    expect(css).toMatch(/\.composerTodoPanel\s*\{/);
    expect(css).toMatch(/\.composerTodoStatus\.completed\s*\{/);
  });

  it("bounds the expanded composer todo panel without hiding the composer", () => {
    const stackRule = ruleBody(css, ".composerStack");
    const panelRule = ruleBody(css, ".composerTodoPanel");

    expect(stackRule).toContain("display: flex;");
    expect(stackRule).toContain("flex-direction: column;");
    expect(panelRule).toContain("max-height: min(32dvh, 260px);");
    expect(panelRule).toContain("overflow: auto;");
    expect(panelRule).toContain("overscroll-behavior: contain;");
  });

  it("lets long todo titles shrink and wrap inside the composer panel item header", () => {
    const itemHeadRule = ruleBody(css, ".composerTodoItemHead");
    const itemTitleRule = ruleBody(css, ".composerTodoItemHead strong");

    expect(itemHeadRule).toContain("min-width: 0;");
    expect(itemTitleRule).toContain("flex: 1 1 auto;");
    expect(itemTitleRule).toContain("min-width: 0;");
    expect(itemTitleRule).toContain("overflow-wrap: anywhere;");
  });

  it("contains long todo summary, description, and status text inside the composer panel", () => {
    const statusRule = ruleBody(css, ".composerTodoStatus");

    expectRuleToContain(css, ".composerTodoSummary", "overflow-wrap: anywhere;");
    expectRuleToContain(css, ".composerTodoItem p", "overflow-wrap: anywhere;");
    expect(statusRule).toContain("flex: 0 1 auto;");
    expect(statusRule).toContain("min-width: 0;");
    expect(statusRule).toContain("overflow-wrap: anywhere;");
  });

  it("keeps the mobile composer todo sizing rules scoped to the 880px media block", () => {
    const mobileRules = mediaBody(css, "(max-width: 880px)");
    const mobileStackRule = ruleBody(mobileRules, ".composerStack");
    const mobilePanelRule = ruleBody(mobileRules, ".composerTodoPanel");

    expect(mobileStackRule).toContain("padding: 8px 10px calc(8px + env(safe-area-inset-bottom));");
    expect(mobilePanelRule).toContain("max-height: min(28dvh, 220px);");
  });
});
