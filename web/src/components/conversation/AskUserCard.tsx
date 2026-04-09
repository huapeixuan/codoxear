import { useEffect, useState } from "preact/hooks";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { MessageEvent, SessionUiRequest } from "../../lib/types";
import { api } from "../../lib/api";
import { useSessionUiStore, useSessionUiStoreApi } from "../../app/providers";

interface AskUserCardProps {
  event: MessageEvent;
  sessionId?: string;
  renderRichText: (value: string, className?: string) => any;
  allowFuzzyLiveMatch?: boolean;
  allowLegacyFallback?: boolean;
}

type OptionInput = { label?: string; value?: string; title?: string; description?: string } | string;
const CUSTOM_RESPONSE_OPTION_RE = /type custom response/i;

function splitAskUserTitle(value: string) {
  const text = value.trim();
  if (!text) return { prompt: "", context: "" };
  const marker = "\n\nContext:\n";
  const index = text.indexOf(marker);
  if (index < 0) return { prompt: text, context: "" };
  return {
    prompt: text.slice(0, index).trim(),
    context: text.slice(index + marker.length).trim(),
  };
}

function normalizeOption(option: OptionInput, index: number) {
  if (typeof option === "string") {
    return { title: option, description: "", value: option, key: option || String(index) };
  }

  const title = option.title ?? option.label ?? option.value ?? `Option ${index + 1}`;
  const value = String(option.value ?? option.title ?? title ?? "");

  return {
    title,
    description: option.description ?? "",
    value,
    key: value || String(index),
  };
}

function askUserRequestId(ev: MessageEvent | SessionUiRequest) {
  if (ev && typeof ev.requestId === "string" && ev.requestId) return ev.requestId;
  if (ev && typeof ev.id === "string" && ev.id) return ev.id;
  if (ev && typeof ev.tool_call_id === "string" && ev.tool_call_id) return ev.tool_call_id;
  return "";
}

function askUserPromptText(ev: MessageEvent | SessionUiRequest) {
  if (ev && typeof ev.question === "string" && ev.question.trim()) return ev.question.trim();
  if (ev && typeof ev.message === "string" && ev.message.trim()) return ev.message.trim();
  if (ev && typeof ev.title === "string" && ev.title.trim()) return splitAskUserTitle(ev.title).prompt;
  return "";
}

function askUserContextText(ev: MessageEvent | SessionUiRequest) {
  if (ev && typeof ev.context === "string" && ev.context.trim()) return ev.context.trim();
  if (ev && typeof ev.title === "string" && ev.title.trim()) return splitAskUserTitle(ev.title).context;
  return "";
}

function askUserOptionSignature(options: Array<OptionInput> | undefined) {
  if (!Array.isArray(options) || !options.length) return "";
  return options
    .map((option, index) => {
      const normalized = normalizeOption(option, index);
      return normalized.title;
    })
    .filter((signature) => !CUSTOM_RESPONSE_OPTION_RE.test(signature))
    .join("\u0001");
}

export function askUserHistorySignature(event: MessageEvent) {
  const prompt = askUserPromptText(event);
  if (!prompt) return "";
  return [prompt, askUserContextText(event), askUserOptionSignature(Array.isArray(event.options) ? event.options : undefined)].join("\u0002");
}

export function isUnresolvedAskUserEvent(event: MessageEvent) {
  return event.type === "ask_user" && !event.resolved && !event.answer && !event.cancelled;
}

function findMatchingLiveRequest(event: MessageEvent, requests: SessionUiRequest[], allowFuzzyLiveMatch: boolean) {
  const directRequestId = askUserRequestId(event);
  const direct = requests.find((request) => askUserRequestId(request) === directRequestId);
  if (direct) return direct;

  if (!allowFuzzyLiveMatch) return undefined;

  const prompt = askUserPromptText(event);
  if (!prompt) return undefined;
  const context = askUserContextText(event);
  const optionSignature = askUserOptionSignature(Array.isArray(event.options) ? event.options : undefined);
  const matches = requests.filter((request) => {
    if (askUserPromptText(request) !== prompt) return false;
    if (askUserContextText(request) !== context) return false;
    return askUserOptionSignature(Array.isArray(request.options) ? request.options : undefined) === optionSignature;
  });
  if (matches.length === 1) return matches[0];
  return undefined;
}

export function AskUserCard({
  event,
  sessionId,
  renderRichText,
  allowFuzzyLiveMatch = true,
  allowLegacyFallback = false,
}: AskUserCardProps) {
  const { requests } = useSessionUiStore();
  const sessionUiStoreApi = useSessionUiStoreApi();

  const liveRequest = findMatchingLiveRequest(event, requests, allowFuzzyLiveMatch);
  const requestId = askUserRequestId(liveRequest ?? event);
  const usingLegacyFallback = Boolean(allowLegacyFallback && !liveRequest && requestId);

  const resolved = Boolean(event.resolved || (event.answer && !liveRequest));
  const allowMultiple = Boolean(liveRequest?.allow_multiple ?? event.allow_multiple);
  const allowFreeform = Boolean(liveRequest?.allow_freeform ?? event.allow_freeform ?? true);

  const options = (
    Array.isArray(liveRequest?.options) && liveRequest.options.length
      ? liveRequest.options
      : Array.isArray(event.options)
        ? event.options
        : []
  ).map(normalizeOption);

  const [selectedValues, setSelectedValues] = useState<string[]>([]);
  const [freeformValue, setFreeformValue] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const [awaitingLogSync, setAwaitingLogSync] = useState(false);

  const prompt = askUserPromptText(liveRequest ?? event) || event.text || "Prompt";
  const context = askUserContextText(liveRequest ?? event);
  const answerText = Array.isArray(event.answer) ? event.answer.join(", ") : event.answer;

  const isConfirm = event.method === "confirm" || liveRequest?.method === "confirm";
  const canAnswer = Boolean(
    sessionId && requestId && !resolved && !submitting && !awaitingLogSync && (liveRequest || usingLegacyFallback)
  );

  useEffect(() => {
    if (resolved || liveRequest) {
      setAwaitingLogSync(false);
    }
  }, [liveRequest, resolved]);

  const handleSubmit = async (values: string[], freeform: string) => {
    if (!sessionId || !requestId) return;

    setSubmitting(true);
    setError("");

    try {
      let finalValue: string | string[] | undefined;
      if (allowMultiple) {
        finalValue = [...values];
        if (freeform.trim()) finalValue.push(freeform.trim());
      } else {
        finalValue = freeform.trim() || values[0];
      }

      const payload = isConfirm
        ? { id: requestId, confirmed: true }
        : { id: requestId, value: finalValue };

      await api.submitUiResponse(sessionId, payload);
      if (usingLegacyFallback) {
        setAwaitingLogSync(true);
      }
      await sessionUiStoreApi.refresh(sessionId, { agentBackend: "pi" });
    } catch (e) {
      setAwaitingLogSync(false);
      setError(e instanceof Error ? e.message : "Failed to submit answer");
    } finally {
      setSubmitting(false);
    }
  };

  const isSelected = (value: string) => selectedValues.includes(value);

  const toggleOption = (value: string) => {
    if (!allowMultiple) {
      void handleSubmit([value], "");
      return;
    }

    setSelectedValues((current) =>
      current.includes(value) ? current.filter((v) => v !== value) : [...current, value]
    );
  };

  return (
    <div data-testid="message-surface" data-kind="ask_user" className={cn("messageCard ask_user", resolved && "resolved")}>
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <Badge variant="outline" className="askUserBadge">Ask user</Badge>
          {event.ts && (
             <time className="text-xs text-muted-foreground">
               {new Date(event.ts * 1000).toLocaleTimeString()}
             </time>
          )}
        </div>

        {context ? renderRichText(context, "askUserContext text-sm text-muted-foreground") : null}

        <div className="askUserQuestion text-base font-semibold">
          {prompt}
        </div>

        {options.length > 0 && (
          <div className="askUserOptions flex flex-wrap gap-2">
            {options.map((option) => (
              <button
                key={option.key}
                type="button"
                disabled={!canAnswer}
                onClick={() => toggleOption(option.value)}
                className={cn(
                  "askUserOption",
                  allowMultiple && "is-multiple",
                  isSelected(option.value) && "is-selected",
                  !canAnswer && "is-disabled"
                )}
              >
                <span className="text-sm font-semibold">{option.title}</span>
                {option.description && (
                  <span className="text-xs opacity-80">{option.description}</span>
                )}
              </button>
            ))}
          </div>
        )}

        {canAnswer && (allowFreeform || allowMultiple || isConfirm) && (
          <div className="askUserComposer flex flex-wrap items-end gap-2">
            {allowFreeform && (
              <Textarea
                value={freeformValue}
                placeholder={allowMultiple ? "Add a custom answer" : "Type your answer"}
                className="askUserFreeformInput min-h-[40px] flex-1"
                onInput={(e) => setFreeformValue(e.currentTarget.value)}
                rows={allowMultiple ? 2 : 1}
              />
            )}
            {(allowMultiple || isConfirm || allowFreeform) && (
              <Button
                size="sm"
                className="askUserSubmit"
                disabled={!isConfirm && !selectedValues.length && !freeformValue.trim()}
                onClick={() => handleSubmit(selectedValues, freeformValue)}
              >
                {submitting ? "Submitting..." : isConfirm ? "Confirm" : allowMultiple ? "Submit selection" : "Submit answer"}
              </Button>
            )}
          </div>
        )}

        {event.cancelled && (
          <div className="askUserAnswer mt-2 text-sm font-medium text-muted-foreground">
            Cancelled
          </div>
        )}

        {answerText && (
          <div className="askUserAnswer mt-2 text-sm font-medium text-amber-800">
            Answer: {answerText}
          </div>
        )}

        {error && (
          <div className="text-xs text-destructive font-medium">
            {error}
          </div>
        )}

        {awaitingLogSync && !error && (
          <div className="text-xs text-muted-foreground">
            Answer sent. Waiting for the session log to confirm it.
          </div>
        )}

        {!resolved && !liveRequest && !usingLegacyFallback && (
          <div className="text-xs text-muted-foreground">
            This prompt is only available in session history, so reply from the active Pi UI.
          </div>
        )}
      </div>
    </div>
  );
}
