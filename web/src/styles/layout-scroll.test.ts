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
  it("defines the editorial shell as a fixed two-column layout with a quiet conversation stack", () => {
    const shellRule = ruleBody(css, ".appShell.editorialShell");
    const sidebarRule = ruleBody(css, ".sidebarColumn");
    const conversationRule = ruleBody(css, ".conversationColumn");
    const paneRule = ruleBody(css, ".conversationPane");

    expect(shellRule).toContain("position: fixed;");
    expect(shellRule).toContain("inset: 0;");
    expect(shellRule).toContain("grid-template-columns: minmax(16rem, var(--sidebar-w)) minmax(0, 1fr);");
    expect(sidebarRule).toContain("min-height: 0;");
    expect(sidebarRule).toContain("overflow: hidden;");
    expect(conversationRule).toContain("display: flex;");
    expect(conversationRule).toContain("flex-direction: column;");
    expect(conversationRule).toContain("min-height: 0;");
    expect(conversationRule).toContain("overflow: hidden;");
    expect(paneRule).toContain("flex: 1 1 0;");
    expect(paneRule).toContain("min-height: 0;");
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

  it("adds bounded workspace dialog hooks and stable toolbar/composer sizing", () => {
    const dialogRule = ruleBody(css, ".workspaceDialog");
    const dialogBodyRule = ruleBody(css, ".workspaceDialogBody");
    const dialogHeaderRule = ruleBody(css, ".workspaceDialogHeader");
    const toolbarTextButtonRule = ruleBody(css, ".toolbarTextButton");
    const composerInputRule = ruleBody(css, ".composerInputWrap");
    const queueButtonRule = ruleBody(css, ".composerQueueButton");
    const sendButtonRule = ruleBody(css, ".sendButton");

    expect(dialogRule).toContain("width: min(72rem, calc(100vw - 32px));");
    expect(dialogRule).toContain("max-height: min(86dvh, 56rem);");
    expect(dialogRule).toContain("overflow: hidden;");
    expect(dialogBodyRule).toContain("min-height: 0;");
    expect(dialogBodyRule).toContain("overflow: hidden;");
    expect(dialogHeaderRule).toContain("flex: 0 0 auto;");
    expect(toolbarTextButtonRule).toContain("min-width: fit-content;");
    expect(composerInputRule).toContain("min-width: 0;");
    expect(queueButtonRule).toContain("min-width: fit-content;");
    expect(sendButtonRule).toContain("width: 44px;");
    expect(sendButtonRule).toContain("height: 44px;");
  });

  it("clamps session title and preview text to two lines so long text does not stretch the rail", () => {
    const titleRule = ruleBody(css, ".sessionTitle");
    const previewRule = ruleBody(css, ".sessionPreview");

    expect(titleRule).toContain("display: -webkit-box;");
    expect(titleRule).toContain("-webkit-line-clamp: 2;");
    expect(titleRule).toContain("-webkit-box-orient: vertical;");
    expect(previewRule).toContain("display: -webkit-box;");
    expect(previewRule).toContain("-webkit-line-clamp: 2;");
    expect(previewRule).toContain("-webkit-box-orient: vertical;");
  });

  it("keeps the mobile composer and shell sizing rules scoped to the 880px media block", () => {
    const mobileRules = mediaBody(css, "(max-width: 880px)");
    const mobileShellRule = ruleBody(mobileRules, ".appShell.editorialShell");
    const mobileStackRule = ruleBody(mobileRules, ".composerStack");
    const mobilePanelRule = ruleBody(mobileRules, ".composerTodoPanel");
    const mobileShellComposerRule = ruleBody(mobileRules, ".composerShell");

    expect(mobileShellRule).toContain("grid-template-columns: 1fr;");
    expect(mobileStackRule).toContain("padding: 8px 10px calc(8px + env(safe-area-inset-bottom));");
    expect(mobilePanelRule).toContain("max-height: min(28dvh, 220px);");
    expect(mobileShellComposerRule).toContain("padding: 8px 10px calc(8px + env(safe-area-inset-bottom));");
  });
});
