import type { ComponentChildren } from "preact";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";

import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

import { useMessagesStore, useSessionsStore } from "../../app/providers";
import type { MessageEvent } from "../../lib/types";

const MAIN_TIMELINE_KINDS = new Set([
  "user",
  "assistant",
  "ask_user",
  "reasoning",
  "tool",
  "tool_result",
  "subagent",
  "todo_snapshot",
  "pi_session",
  "pi_model_change",
  "pi_thinking_level_change",
  "pi_event",
  "event",
]);

const CHAT_GROUPABLE_KINDS = new Set(["user", "assistant", "ask_user"]);
const COLLAPSIBLE_LINE_THRESHOLD = 8;
const COLLAPSIBLE_CHAR_THRESHOLD = 420;

const EVENT_LABELS: Record<string, string> = {
  ask_user: "Question",
  reasoning: "Reasoning",
  tool: "Tool",
  tool_result: "Tool Result",
  subagent: "Subagent",
  todo_snapshot: "Todo Progress",
  pi_session: "Session",
  pi_model_change: "Model Change",
  pi_thinking_level_change: "Thinking Level",
  pi_event: "System Event",
  event: "Event",
};

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function renderInlineMarkdown(value: string): string {
  return value
    .split(/(`[^`]+`)/g)
    .filter((part) => part.length > 0)
    .map((part) => {
      if (part.startsWith("`") && part.endsWith("`") && part.length >= 2) {
        return `<code>${escapeHtml(part.slice(1, -1))}</code>`;
      }
      return escapeHtml(part);
    })
    .join("");
}

function renderMessageHtml(value: string): string {
  const normalized = value.replace(/\r\n?/g, "\n");
  const blocks: string[] = [];
  const fenceRegex = /```(?:[\w-]+)?\n([\s\S]*?)```/g;
  let lastIndex = 0;

  for (const match of normalized.matchAll(fenceRegex)) {
    const index = match.index ?? 0;
    const before = normalized.slice(lastIndex, index);
    if (before.trim()) {
      for (const paragraph of before.split(/\n\n+/).filter(Boolean)) {
        blocks.push(`<p>${renderInlineMarkdown(paragraph).replace(/\n/g, "<br />")}</p>`);
      }
    }
    blocks.push(`<pre><code>${escapeHtml((match[1] || "").replace(/\n$/, ""))}</code></pre>`);
    lastIndex = index + match[0].length;
  }

  const tail = normalized.slice(lastIndex);
  if (tail.trim()) {
    for (const paragraph of tail.split(/\n\n+/).filter(Boolean)) {
      blocks.push(`<p>${renderInlineMarkdown(paragraph).replace(/\n/g, "<br />")}</p>`);
    }
  }

  return blocks.join("");
}

function messageContentParts(event: MessageEvent): string[] {
  const message = event.message;
  if (!message || !Array.isArray(message.content)) {
    return [];
  }
  return message.content
    .map((item) => (typeof item?.text === "string" ? item.text.trim() : ""))
    .filter(Boolean);
}

function firstNonEmptyText(...values: Array<unknown>): string {
  for (const value of values) {
    if (typeof value === "string" && value.trim()) {
      return value.trim();
    }
  }
  return "";
}

function askUserAnswerText(event: MessageEvent): string {
  if (event.cancelled) {
    return "Cancelled";
  }
  if (Array.isArray(event.answer)) {
    const answers = event.answer.filter((item): item is string => typeof item === "string" && item.trim().length > 0);
    if (answers.length) {
      return `Answer: ${answers.join(", ")}`;
    }
  }
  if (typeof event.answer === "string" && event.answer.trim()) {
    return `Answer: ${event.answer.trim()}`;
  }
  return "";
}

function detailsSummary(details: Record<string, unknown> | undefined): string {
  if (!details) {
    return "";
  }
  if (typeof details.summary === "string" && details.summary.trim()) {
    return details.summary.trim();
  }
  if (typeof details.error === "string" && details.error.trim()) {
    return details.error.trim();
  }
  if (Array.isArray(details.todos) && details.todos.length) {
    return `${details.todos.length} todo item${details.todos.length === 1 ? "" : "s"}`;
  }
  const keys = Object.keys(details);
  if (keys.length) {
    return `Details: ${keys.join(", ")}`;
  }
  return "";
}

function contentTextFromMessage(event: MessageEvent): string {
  const kind = eventKind(event);
  if (kind === "ask_user") {
    return firstNonEmptyText(event.question, event.text, "Prompt");
  }
  if (typeof event.text === "string" && event.text.trim()) {
    return event.text;
  }
  const contentParts = messageContentParts(event);
  if (contentParts.length) {
    return contentParts.join("\n");
  }
  if (typeof event.output === "string" && event.output.trim()) {
    return event.output;
  }
  if (typeof event.summary === "string" && event.summary.trim()) {
    return event.summary;
  }
  if (typeof event.question === "string" && event.question.trim()) {
    return event.question;
  }
  if (typeof event.context === "string" && event.context.trim()) {
    return event.context;
  }
  if (event.details) {
    return detailsSummary(event.details) || JSON.stringify(event.details, null, 2);
  }
  return JSON.stringify(event, null, 2);
}

function eventKind(event: MessageEvent): string {
  if (typeof event.role === "string" && event.role) {
    return event.role;
  }
  if (typeof event.message?.role === "string" && event.message.role) {
    return event.message.role;
  }
  if (event.toolName === "ask_user") {
    return "ask_user";
  }
  return typeof event.type === "string" && event.type ? event.type : "event";
}

function shouldRenderInMainConversation(event: MessageEvent): boolean {
  const kind = eventKind(event);
  if (MAIN_TIMELINE_KINDS.has(kind)) {
    return true;
  }
  return Boolean(firstNonEmptyText(event.text, event.summary, event.question, event.context));
}

function canGroupEvent(kind: string): boolean {
  return CHAT_GROUPABLE_KINDS.has(kind);
}

function eventLabel(kind: string): string {
  return EVENT_LABELS[kind] || kind.replace(/_/g, " ").replace(/\b\w/g, (char) => char.toUpperCase());
}

function surfaceBadgeVariant(kind: string): "default" | "secondary" | "outline" {
  switch (kind) {
    case "user":
      return "default";
    case "assistant":
    case "tool_result":
    case "todo_snapshot":
      return "secondary";
    default:
      return "outline";
  }
}

function messageSurfaceTone(kind: string, isError = false): string {
  if (isError) {
    return "border-destructive/40 bg-destructive/5";
  }

  switch (kind) {
    case "user":
      return "border-primary/30 bg-primary/10 text-primary-foreground/90";
    case "assistant":
      return "border-border/70 bg-card/95";
    case "ask_user":
      return "border-amber-300/70 bg-amber-50/90";
    case "reasoning":
      return "border-sky-200/80 bg-sky-50/80";
    case "tool":
      return "border-indigo-200/80 bg-indigo-50/80";
    case "tool_result":
      return "border-emerald-200/80 bg-emerald-50/80";
    case "subagent":
      return "border-slate-200/80 bg-slate-50/85";
    case "todo_snapshot":
      return "border-teal-200/80 bg-teal-50/85";
    default:
      return "border-border/60 bg-muted/30";
  }
}

function renderRichText(value: string, className = "messageBody") {
  if (!value.trim()) {
    return null;
  }
  return <div className={className} dangerouslySetInnerHTML={{ __html: renderMessageHtml(value) }} />;
}

function shouldCollapseContent(value: string): boolean {
  const normalized = value.trim();
  if (!normalized) {
    return false;
  }
  const lineCount = normalized.split("\n").length;
  return lineCount > COLLAPSIBLE_LINE_THRESHOLD || normalized.length > COLLAPSIBLE_CHAR_THRESHOLD;
}

function ExpandableRichText({
  value,
  className = "messageBody",
}: {
  value: string;
  className?: string;
}) {
  const collapsible = shouldCollapseContent(value);
  const [expanded, setExpanded] = useState(false);
  const previousValueRef = useRef(value);
  const contentClassName = cn("messageExpandableContent", collapsible && !expanded && "isCollapsed");

  useEffect(() => {
    if (previousValueRef.current !== value) {
      previousValueRef.current = value;
      setExpanded(false);
    }
  }, [value]);

  return (
    <div className="messageExpandable space-y-3">
      <div className={contentClassName}>{renderRichText(value, className)}</div>
      {collapsible ? (
        <button
          type="button"
          className="messageExpandButton inline-flex items-center rounded-full border border-border/70 px-3 py-1 text-xs font-medium text-muted-foreground transition hover:bg-accent hover:text-accent-foreground"
          aria-expanded={expanded ? "true" : "false"}
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      ) : null}
    </div>
  );
}

function renderCardHeader(kind: string, title?: string, summary?: string) {
  return (
    <header className="messageCardHeader flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={surfaceBadgeVariant(kind)}>{eventLabel(kind)}</Badge>
        {title ? <div className="messageCardTitle text-sm font-semibold text-foreground">{title}</div> : null}
      </div>
      {summary ? <div className="messageCardSummary text-sm text-muted-foreground">{summary}</div> : null}
    </header>
  );
}

function MessageSurface({
  kind,
  children,
  grouped = false,
  isError = false,
  className,
}: {
  kind: string;
  children: ComponentChildren;
  grouped?: boolean;
  isError?: boolean;
  className?: string;
}) {
  const isChatSurface = kind === "user" || kind === "assistant" || kind === "ask_user";

  return (
    <Card
      data-testid="message-surface"
      data-kind={kind}
      className={cn(
        "messageSurface rounded-[1.35rem] border shadow-sm backdrop-blur-sm transition-colors",
        isChatSurface ? "max-w-3xl" : "max-w-4xl",
        kind === "user" ? "ml-auto messageBubble user" : undefined,
        kind === "assistant" ? "mr-auto messageBubble assistant" : undefined,
        kind === "ask_user" ? "mr-auto messageBubble messageCard ask_user" : undefined,
        !isChatSurface ? "messageCard" : undefined,
        grouped && "grouped",
        isError && "isError",
        messageSurfaceTone(kind, isError),
        className,
      )}
    >
      <CardContent className="space-y-3 p-4">{children}</CardContent>
    </Card>
  );
}

function renderChatCard(event: MessageEvent, kind: "user" | "assistant") {
  const label = kind === "user" ? "You" : "Assistant";

  return (
    <MessageSurface kind={kind}>
      {renderCardHeader(kind, label)}
      <div className="messageBody prose prose-sm max-w-none" dangerouslySetInnerHTML={{ __html: renderMessageHtml(contentTextFromMessage(event)) }} />
    </MessageSurface>
  );
}

function renderAskUserCard(event: MessageEvent) {
  const prompt = firstNonEmptyText(event.question, event.text, "Prompt");
  const answer = askUserAnswerText(event);
  const context = typeof event.context === "string" ? event.context.trim() : "";

  return (
    <MessageSurface kind="ask_user">
      {renderCardHeader("ask_user", prompt)}
      {context ? renderRichText(context, "messageCardContext text-sm text-muted-foreground") : null}
      {answer ? <div className="messageCardFooterText text-sm font-medium text-foreground/80">{answer}</div> : null}
    </MessageSurface>
  );
}

function renderReasoningCard(event: MessageEvent) {
  const summary = firstNonEmptyText(event.summary);
  const body = firstNonEmptyText(event.text, summary);

  return (
    <MessageSurface kind="reasoning">
      {renderCardHeader("reasoning", undefined, summary && summary !== body ? summary : undefined)}
      {body ? <ExpandableRichText key={body} value={body} /> : null}
    </MessageSurface>
  );
}

function renderToolCard(event: MessageEvent) {
  const body = firstNonEmptyText(event.text, event.summary, event.context);

  return (
    <MessageSurface kind="tool">
      {renderCardHeader("tool", firstNonEmptyText(event.name, "Unnamed tool"))}
      {body ? renderRichText(body) : null}
    </MessageSurface>
  );
}

function renderToolResultCard(event: MessageEvent) {
  const body = firstNonEmptyText(event.text, detailsSummary(event.details));
  const detailsText = !event.text && event.details ? JSON.stringify(event.details, null, 2) : "";

  return (
    <MessageSurface kind="tool_result" isError={Boolean(event.is_error)}>
      {renderCardHeader("tool_result", firstNonEmptyText(event.name, "Tool result"))}
      {body ? <ExpandableRichText key={body} value={body} /> : null}
      {detailsText ? <pre className="messageCardPre overflow-x-auto rounded-xl bg-background/80 p-3 text-sm">{detailsText}</pre> : null}
    </MessageSurface>
  );
}

function renderSubagentCard(event: MessageEvent) {
  const output = firstNonEmptyText(event.output, event.text);
  const pending = !output;

  return (
    <MessageSurface kind="subagent">
      {renderCardHeader("subagent", firstNonEmptyText(event.agent, "subagent"), firstNonEmptyText(event.task))}
      <div className="messageMetaList grid gap-2 sm:grid-cols-3">
        {event.agent ? <div className="messageMetaItem rounded-xl bg-background/70 p-3 text-sm"><span className="block text-xs uppercase tracking-wide text-muted-foreground">Agent</span><strong>{event.agent}</strong></div> : null}
        {event.task ? <div className="messageMetaItem rounded-xl bg-background/70 p-3 text-sm"><span className="block text-xs uppercase tracking-wide text-muted-foreground">Task</span><strong>{event.task}</strong></div> : null}
        <div className="messageMetaItem rounded-xl bg-background/70 p-3 text-sm"><span className="block text-xs uppercase tracking-wide text-muted-foreground">Status</span><strong>{pending ? "Running" : "Completed"}</strong></div>
      </div>
      {output ? renderRichText(output) : <div className="messageCardFooterText text-sm text-muted-foreground">Waiting for subagent output...</div>}
    </MessageSurface>
  );
}

function renderTodoSnapshotCard(event: MessageEvent) {
  const items = Array.isArray(event.items) ? event.items.slice(0, 3) : [];

  return (
    <MessageSurface kind="todo_snapshot">
      {renderCardHeader("todo_snapshot", firstNonEmptyText(event.progress_text, "Todo snapshot"), firstNonEmptyText(event.operation))}
      {items.length ? (
        <ul className="messageTodoList space-y-2">
          {items.map((item, index) => (
            <li key={`${item.title || "todo"}-${index}`} className="messageTodoItem flex items-start gap-3 rounded-xl bg-background/70 px-3 py-2 text-sm">
              <span className={cn("messageTodoStatus rounded-full px-2 py-0.5 text-xs font-semibold uppercase tracking-wide", typeof item.status === "string" ? item.status : "unknown")}>{item.status || "unknown"}</span>
              <span>{item.title || item.description || "Untitled item"}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {event.text ? renderRichText(event.text) : null}
    </MessageSurface>
  );
}

function renderSystemCard(event: MessageEvent, kind: string) {
  const title = firstNonEmptyText(event.summary, event.name);
  const body = firstNonEmptyText(event.text, event.context, event.question, title === event.summary ? "" : event.summary);

  return (
    <MessageSurface kind={kind}>
      {renderCardHeader(kind, title || undefined)}
      {body ? renderRichText(body) : null}
    </MessageSurface>
  );
}

function renderConversationEvent(event: MessageEvent, kind: string) {
  switch (kind) {
    case "user":
    case "assistant":
      return renderChatCard(event, kind);
    case "ask_user":
      return renderAskUserCard(event);
    case "reasoning":
      return renderReasoningCard(event);
    case "tool":
      return renderToolCard(event);
    case "tool_result":
      return renderToolResultCard(event);
    case "subagent":
      return renderSubagentCard(event);
    case "todo_snapshot":
      return renderTodoSnapshotCard(event);
    case "pi_session":
    case "pi_model_change":
    case "pi_thinking_level_change":
    case "pi_event":
    case "event":
      return renderSystemCard(event, kind);
    default:
      return renderSystemCard(event, kind);
  }
}

function renderLoadingCards() {
  return (
    <div className="messageList flex flex-col gap-3">
      {Array.from({ length: 3 }, (_value, index) => (
        <div key={index} className={cn("messageRow flex", index === 0 ? "assistant" : index === 1 ? "tool" : "assistant")}>
          <Card data-testid="message-surface" data-kind="loading" className="messageSurface max-w-4xl rounded-[1.35rem] border border-border/60 bg-card/90 shadow-sm">
            <CardContent className="space-y-3 p-4">
              <div className="flex items-center gap-2">
                <Skeleton className="h-5 w-20 rounded-full" />
                <Skeleton className="h-4 w-36" />
              </div>
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-5/6" />
              {index === 1 ? <Skeleton className="h-16 w-full rounded-xl" /> : null}
            </CardContent>
          </Card>
        </div>
      ))}
    </div>
  );
}

const messageRowIds = new WeakMap<object, string>();
let nextMessageRowId = 0;

function messageRowKey(event: MessageEvent, kind: string, index: number) {
  const row = event as Record<string, unknown>;
  const messageId = typeof row.message_id === "string" ? row.message_id.trim() : "";
  if (messageId) {
    return `${kind}:${messageId}`;
  }
  if (event && typeof event === "object") {
    let objectKey = messageRowIds.get(event as object);
    if (!objectKey) {
      nextMessageRowId += 1;
      objectKey = `row-${nextMessageRowId}`;
      messageRowIds.set(event as object, objectKey);
    }
    return `${kind}:${objectKey}`;
  }
  return `${kind}:fallback-${index}`;
}

function scrollPaneToBottom(element: HTMLElement) {
  if (typeof element.scrollTo === "function") {
    element.scrollTo({ top: element.scrollHeight });
    return;
  }
  element.scrollTop = element.scrollHeight;
}

export function ConversationPane() {
  const { activeSessionId } = useSessionsStore();
  const { bySessionId, loading } = useMessagesStore();
  const rawMessages = activeSessionId ? bySessionId[activeSessionId] ?? [] : [];
  const messages = rawMessages.filter(shouldRenderInMainConversation);
  const sectionRef = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    const pane = sectionRef.current?.querySelector(".conversationPane") as HTMLElement | null;
    if (!pane || !messages.length) {
      return;
    }
    scrollPaneToBottom(pane);
  }, [messages.length, activeSessionId]);

  return (
    <section ref={sectionRef} className="conversationTimeline flex min-h-0 flex-1">
      <ScrollArea className={cn("conversationPane conversationScrollArea min-h-0 flex-1 px-3 py-4", !activeSessionId && "emptyState")}>
        {loading ? (
          renderLoadingCards()
        ) : (
          <div className="messageList flex flex-col gap-3">
            {messages.length ? (
              messages.map((message, index) => {
                const kind = eventKind(message);
                const prevKind = index > 0 ? eventKind(messages[index - 1]) : null;
                const grouped = prevKind === kind && canGroupEvent(kind);
                return (
                  <div key={messageRowKey(message, kind, index)} className={cn("messageRow flex", kind, grouped && "grouped")}>
                    {renderConversationEvent(message, kind)}
                  </div>
                );
              })
            ) : (
              <Card className="rounded-[1.35rem] border-dashed border-border/60 bg-muted/20 shadow-none">
                <CardContent className="p-6 text-sm text-muted-foreground">
                  {activeSessionId ? "No conversation events yet." : "Select a session to see its conversation timeline."}
                </CardContent>
              </Card>
            )}
          </div>
        )}
      </ScrollArea>
    </section>
  );
}
