import { memo } from "preact/compat";
import type { JSX } from "preact";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";

import type { SessionUiRequest } from "../../lib/types";

type DraftValue = string | string[];
type OptionInput = { label?: string; value?: string; title?: string; description?: string } | string;
type AskUserBridgeQuestion = {
  header: string;
  question: string;
  options: Array<{ label: string; description?: string; preview?: string }>;
  multiSelect?: boolean;
};
type AskUserBridgeRequest = {
  questions: AskUserBridgeQuestion[];
  metadata?: Record<string, unknown>;
};
type AskUserBridgeAnswers = Record<string, string | string[]>;

const ASK_USER_BRIDGE_PREFIX = "__codoxear_ask_user_bridge_v1__";

function normalizeOption(option: OptionInput, index: number) {
  if (typeof option === "string") {
    return { label: option, description: "", value: option, key: option || String(index) };
  }

  const label = option.label ?? option.title ?? option.value ?? `Option ${index + 1}`;
  const value = String(option.value ?? option.title ?? label ?? "");

  return {
    label,
    description: option.description ?? "",
    value,
    key: value || String(index),
  };
}

function parseAskUserBridgeRequest(request: SessionUiRequest): AskUserBridgeRequest | null {
  if (request.method !== "editor") {
    return null;
  }

  const prefill = typeof request.prefill === "string" ? request.prefill : "";
  if (!prefill.startsWith(`${ASK_USER_BRIDGE_PREFIX}\n`)) {
    return null;
  }

  try {
    const parsed = JSON.parse(prefill.slice(ASK_USER_BRIDGE_PREFIX.length + 1)) as {
      questions?: unknown;
      metadata?: Record<string, unknown>;
    };
    if (!Array.isArray(parsed.questions) || !parsed.questions.length) {
      return null;
    }

    const questions: AskUserBridgeQuestion[] = [];
    for (const question of parsed.questions) {
      if (!question || typeof question !== "object") {
        continue;
      }
      const row = question as Record<string, unknown>;
      const header = typeof row.header === "string" ? row.header.trim() : "";
      const prompt = typeof row.question === "string" ? row.question.trim() : "";
      const options: AskUserBridgeQuestion["options"] = [];
      if (Array.isArray(row.options)) {
        for (const option of row.options) {
          if (!option || typeof option !== "object") {
            continue;
          }
          const value = option as Record<string, unknown>;
          const label = typeof value.label === "string" ? value.label.trim() : "";
          if (!label) {
            continue;
          }
          options.push({
            label,
            description: typeof value.description === "string" ? value.description.trim() : undefined,
            preview: typeof value.preview === "string" ? value.preview : undefined,
          });
        }
      }

      if (!header || !prompt || !options.length) {
        continue;
      }

      questions.push({
        header,
        question: prompt,
        options,
        multiSelect: row.multiSelect === true,
      });
    }

    if (!questions.length) {
      return null;
    }

    return { questions, metadata: parsed.metadata };
  } catch {
    return null;
  }
}

function encodeAskUserBridgeResponse(answers: AskUserBridgeAnswers) {
  return `${ASK_USER_BRIDGE_PREFIX}\n${JSON.stringify({ action: "answered", answers })}`;
}

function getInitialDraftValue(request: SessionUiRequest): DraftValue {
  if (Array.isArray(request.value)) {
    return request.value.filter((item): item is string => typeof item === "string");
  }
  if (typeof request.value === "string") {
    return request.value;
  }
  if (request.method === "select" && Array.isArray(request.options) && request.options.length > 0 && !request.allow_multiple) {
    return normalizeOption(request.options[0], 0).value;
  }
  return request.allow_multiple ? [] : "";
}

function normalizeRequestValue(request: SessionUiRequest, draftValue: DraftValue): string | string[] | undefined {
  if (request.method === "confirm") {
    return undefined;
  }
  if (request.allow_multiple) {
    return Array.isArray(draftValue) ? draftValue : draftValue ? [draftValue] : [];
  }
  return Array.isArray(draftValue) ? draftValue[0] ?? "" : draftValue;
}

function getRequestHeading(request: SessionUiRequest): string {
  return request.title || request.label || request.question || request.method || "Request";
}

function getRequestBody(request: SessionUiRequest): string {
  return request.message || request.context || "";
}

function mergeFreeformValue(request: SessionUiRequest, normalizedValue: string | string[] | undefined, freeformValue: string) {
  const trimmedFreeform = freeformValue.trim();

  if (!trimmedFreeform) {
    return normalizedValue;
  }

  if (request.allow_multiple) {
    const existingValues = Array.isArray(normalizedValue)
      ? normalizedValue
      : normalizedValue
        ? [normalizedValue]
        : [];
    return [...existingValues, trimmedFreeform];
  }

  return trimmedFreeform;
}

function SelectField(props: JSX.SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
      {...props}
    />
  );
}

export interface WorkspaceRequestCardProps {
  request: SessionUiRequest;
  sessionId: string | null;
  draftValue?: DraftValue;
  freeformValue?: string;
  askUserBridgeAnswers?: AskUserBridgeAnswers;
  submitting: boolean;
  errorMessage?: string;
  onDraftChange(value: DraftValue): void;
  onFreeformChange(value: string): void;
  onAskUserBridgeAnswerChange(question: string, value: string | string[]): void;
  onConfirm(payload: Record<string, unknown>): void;
  onCancel(payload: Record<string, unknown>): void;
}

function sameDraftValue(left: DraftValue | undefined, right: DraftValue | undefined) {
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) {
      return false;
    }
    for (let index = 0; index < left.length; index += 1) {
      if (left[index] !== right[index]) {
        return false;
      }
    }
    return true;
  }
  return left === right;
}

function sameAskUserBridgeAnswers(left: AskUserBridgeAnswers | undefined, right: AskUserBridgeAnswers | undefined) {
  const leftKeys = Object.keys(left ?? {});
  const rightKeys = Object.keys(right ?? {});
  if (leftKeys.length !== rightKeys.length) {
    return false;
  }
  for (const key of leftKeys) {
    const leftValue = left?.[key];
    const rightValue = right?.[key];
    if (!sameDraftValue(leftValue, rightValue)) {
      return false;
    }
  }
  return true;
}

export const WorkspaceRequestCard = memo(function WorkspaceRequestCard({
  request,
  sessionId,
  draftValue,
  freeformValue = "",
  askUserBridgeAnswers = {},
  submitting,
  errorMessage = "",
  onDraftChange,
  onFreeformChange,
  onAskUserBridgeAnswerChange,
  onConfirm,
  onCancel,
}: WorkspaceRequestCardProps) {
  const askUserBridge = parseAskUserBridgeRequest(request);
  const effectiveDraftValue = draftValue ?? getInitialDraftValue(request);
  const options = Array.isArray(request.options) ? request.options : [];
  const bodyText = getRequestBody(request);
  const selectedValues = Array.isArray(effectiveDraftValue) ? effectiveDraftValue : [];
  const askUserBridgeReady = Boolean(
    askUserBridge && askUserBridge.questions.every((question) => {
      const answer = askUserBridgeAnswers[question.question];
      return Array.isArray(answer) ? answer.length > 0 : typeof answer === "string" && answer.trim().length > 0;
    }),
  );

  return (
    <Card className="rounded-[1.2rem] border-border/70 bg-background/75 shadow-sm">
      <CardContent className="space-y-4 p-4">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="secondary">{request.method || "request"}</Badge>
            {!askUserBridge && request.allow_multiple ? <Badge variant="outline">multi-select</Badge> : null}
            {!askUserBridge && request.allow_freeform ? <Badge variant="outline">freeform</Badge> : null}
          </div>
          <div>
            <h3 className="text-sm font-semibold text-foreground">{askUserBridge ? "AskUserQuestion" : getRequestHeading(request)}</h3>
            {bodyText ? <p className="mt-1 text-sm text-muted-foreground">{bodyText}</p> : null}
          </div>
        </div>

        {askUserBridge ? (
          <div className="space-y-4">
            {askUserBridge.questions.map((question) => {
              const currentAnswer = askUserBridgeAnswers[question.question];
              const selectedAnswer = Array.isArray(currentAnswer)
                ? currentAnswer
                : typeof currentAnswer === "string"
                  ? [currentAnswer]
                  : [];
              return (
                <section key={question.question} className="space-y-3 rounded-2xl border border-border/60 bg-card/60 p-3">
                  <div>
                    <p className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">{question.header}</p>
                    <h4 className="text-sm font-semibold text-foreground">{question.question}</h4>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    {question.options.map((option) => {
                      const isSelected = selectedAnswer.includes(option.label);
                      return (
                        <Button
                          key={`${question.question}-${option.label}`}
                          type="button"
                          variant={isSelected ? "default" : "outline"}
                          className="h-auto min-h-10 rounded-full px-4 py-2 text-left"
                          onClick={() => {
                            const previous = askUserBridgeAnswers[question.question];
                            const previousValues = Array.isArray(previous)
                              ? previous
                              : typeof previous === "string"
                                ? [previous]
                                : [];
                            const nextValue = question.multiSelect
                              ? previousValues.includes(option.label)
                                ? previousValues.filter((value) => value !== option.label)
                                : [...previousValues, option.label]
                              : option.label;
                            onAskUserBridgeAnswerChange(question.question, nextValue);
                          }}
                        >
                          <span className="flex flex-col items-start gap-1">
                            <span>{option.label}</span>
                            {option.description ? <span className="text-xs font-normal text-muted-foreground">{option.description}</span> : null}
                          </span>
                        </Button>
                      );
                    })}
                  </div>
                </section>
              );
            })}
          </div>
        ) : null}

        {!askUserBridge && (request.method === "confirm" ? null : options.length ? (
          request.allow_multiple ? (
            <div className="space-y-2">
              {options.map((option, optionIndex) => {
                const normalized = normalizeOption(option, optionIndex);
                return (
                  <label key={normalized.key} className="workspaceOption flex cursor-pointer gap-3 rounded-2xl border border-border/60 bg-card/60 px-3 py-3 text-sm">
                    <input
                      type="checkbox"
                      checked={selectedValues.includes(normalized.value)}
                      onInput={(event) => {
                        const checked = event.currentTarget.checked;
                        const existing = Array.isArray(effectiveDraftValue) ? effectiveDraftValue : [];
                        const next = checked
                          ? [...existing, normalized.value]
                          : existing.filter((value) => value !== normalized.value);
                        onDraftChange(next);
                      }}
                    />
                    <span className="space-y-1">
                      <span className="block font-medium text-foreground">{normalized.label}</span>
                      {normalized.description ? <span className="block text-muted-foreground">{normalized.description}</span> : null}
                    </span>
                  </label>
                );
              })}
            </div>
          ) : (
            <SelectField
              value={Array.isArray(effectiveDraftValue) ? effectiveDraftValue[0] ?? "" : effectiveDraftValue}
              onInput={(event) => onDraftChange(event.currentTarget.value)}
            >
              {options.map((option, optionIndex) => {
                const normalized = normalizeOption(option, optionIndex);
                return (
                  <option key={normalized.key} value={normalized.value}>
                    {normalized.label}
                  </option>
                );
              })}
            </SelectField>
          )
        ) : (
          <Textarea
            value={Array.isArray(effectiveDraftValue) ? effectiveDraftValue.join("\n") : effectiveDraftValue}
            className="min-h-[8rem] rounded-2xl border-border/70 bg-background/80"
            onInput={(event) => onDraftChange(event.currentTarget.value)}
          />
        ))}

        {!askUserBridge && request.allow_freeform ? (
          <div className="space-y-2">
            <label className="text-sm font-medium text-foreground">Other response</label>
            <Textarea
              value={freeformValue}
              placeholder="Other response"
              className="min-h-[6rem] rounded-2xl border-border/70 bg-background/80"
              onInput={(event) => onFreeformChange(event.currentTarget.value)}
            />
          </div>
        ) : null}

        {sessionId ? (
          <div className="space-y-2">
            <div className="formActions gap-2">
              <Button
                type="button"
                disabled={submitting || Boolean(askUserBridge && !askUserBridgeReady)}
                onClick={() => {
                  const payload = askUserBridge
                    ? {
                        id: request.id,
                        value: encodeAskUserBridgeResponse(askUserBridgeAnswers),
                      }
                    : request.method === "confirm"
                      ? { id: request.id, confirmed: true }
                      : {
                          id: request.id,
                          value: mergeFreeformValue(request, normalizeRequestValue(request, effectiveDraftValue), freeformValue),
                        };
                  onConfirm(payload);
                }}
              >
                {submitting ? "Submitting..." : "Confirm"}
              </Button>
              <Button
                type="button"
                variant="outline"
                disabled={submitting}
                onClick={() => onCancel({ id: request.id, cancelled: true })}
              >
                Cancel
              </Button>
            </div>
            {errorMessage ? <p className="text-sm font-medium text-destructive">{errorMessage}</p> : null}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}, (prev, next) => {
  return prev.request === next.request
    && prev.sessionId === next.sessionId
    && sameDraftValue(prev.draftValue, next.draftValue)
    && prev.freeformValue === next.freeformValue
    && sameAskUserBridgeAnswers(prev.askUserBridgeAnswers, next.askUserBridgeAnswers)
    && prev.submitting === next.submitting
    && prev.errorMessage === next.errorMessage;
});
