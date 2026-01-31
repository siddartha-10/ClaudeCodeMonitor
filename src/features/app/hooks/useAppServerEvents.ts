import { useEffect } from "react";
import type { AppServerEvent, PermissionDenial, RequestUserInputRequest } from "../../../types";
import { subscribeAppServerEvents } from "../../../services/events";

type AgentDelta = {
  workspaceId: string;
  threadId: string;
  itemId: string;
  delta: string;
};

type AgentCompleted = {
  workspaceId: string;
  threadId: string;
  itemId: string;
  text: string;
  model?: string | null;
};

type AppServerEventHandlers = {
  onWorkspaceConnected?: (workspaceId: string) => void;
  onPermissionDenied?: (event: {
    workspaceId: string;
    threadId: string;
    turnId: string;
    denials: PermissionDenial[];
  }) => void;
  onRequestUserInput?: (request: RequestUserInputRequest) => void;
  onAgentMessageDelta?: (event: AgentDelta) => void;
  onAgentMessageStarted?: (event: {
    workspaceId: string;
    threadId: string;
    itemId: string;
    model?: string | null;
  }) => void;
  onAgentMessageCompleted?: (event: AgentCompleted) => void;
  onAppServerEvent?: (event: AppServerEvent) => void;
  onTurnStarted?: (workspaceId: string, threadId: string, turnId: string) => void;
  onTurnCompleted?: (workspaceId: string, threadId: string, turnId: string) => void;
  onContextCompacted?: (workspaceId: string, threadId: string, turnId: string) => void;
  onTurnError?: (
    workspaceId: string,
    threadId: string,
    turnId: string,
    payload: { message: string; willRetry: boolean },
  ) => void;
  onTurnPlanUpdated?: (
    workspaceId: string,
    threadId: string,
    turnId: string,
    payload: { explanation: unknown; plan: unknown },
  ) => void;
  onThreadCreated?: (workspaceId: string, thread: Record<string, unknown>) => void;
  onItemStarted?: (workspaceId: string, threadId: string, item: Record<string, unknown>) => void;
  onItemCompleted?: (workspaceId: string, threadId: string, item: Record<string, unknown>) => void;
  onReasoningSummaryDelta?: (workspaceId: string, threadId: string, itemId: string, delta: string) => void;
  onReasoningSummaryBoundary?: (workspaceId: string, threadId: string, itemId: string) => void;
  onReasoningTextDelta?: (workspaceId: string, threadId: string, itemId: string, delta: string) => void;
  onCommandOutputDelta?: (workspaceId: string, threadId: string, itemId: string, delta: string) => void;
  onTerminalInteraction?: (
    workspaceId: string,
    threadId: string,
    itemId: string,
    stdin: string,
  ) => void;
  onFileChangeOutputDelta?: (workspaceId: string, threadId: string, itemId: string, delta: string) => void;
  onTurnDiffUpdated?: (workspaceId: string, threadId: string, diff: string) => void;
  onThreadTokenUsageUpdated?: (
    workspaceId: string,
    threadId: string,
    tokenUsage: Record<string, unknown>,
  ) => void;
  };

export function useAppServerEvents(handlers: AppServerEventHandlers) {
  useEffect(() => {
    const unlisten = subscribeAppServerEvents((payload) => {
      handlers.onAppServerEvent?.(payload);

      const { workspace_id, message } = payload;
      const method = String(message.method ?? "");

      if (method === "claude/connected") {
        handlers.onWorkspaceConnected?.(workspace_id);
        return;
      }

      if (method === "item/tool/requestUserInput" && typeof message.id === "number") {
        const params = (message.params as Record<string, unknown>) ?? {};
        const questionsRaw = Array.isArray(params.questions) ? params.questions : [];
        const questions = questionsRaw
          .filter(
            (entry): entry is Record<string, unknown> =>
              entry !== null && typeof entry === "object",
          )
          .map((question) => {
            const optionsRaw = Array.isArray(question.options) ? question.options : [];
            const options = optionsRaw
              .filter(
                (option): option is Record<string, unknown> =>
                  option !== null && typeof option === "object",
              )
              .map((record) => {
                const label = String(record.label ?? "").trim();
                const description = String(record.description ?? "").trim();
                if (!label && !description) {
                  return null;
                }
                return { label, description };
              })
              .filter((option): option is { label: string; description: string } => Boolean(option));
            return {
              id: String(question.id ?? "").trim(),
              header: String(question.header ?? ""),
              question: String(question.question ?? ""),
              isOther: Boolean(question.isOther ?? question.is_other),
              options: options.length ? options : undefined,
            };
          })
          .filter((question) => question.id);
        handlers.onRequestUserInput?.({
          workspace_id,
          request_id: message.id,
          params: {
            thread_id: String(params.threadId ?? params.thread_id ?? ""),
            turn_id: String(params.turnId ?? params.turn_id ?? ""),
            item_id: String(params.itemId ?? params.item_id ?? ""),
            tool_use_id: String(params.toolUseId ?? params.tool_use_id ?? ""),
            questions,
          },
        });
        return;
      }

      if (method === "item/agentMessage/delta") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const itemId = String(params.itemId ?? params.item_id ?? "");
        const delta = String(params.delta ?? "");
        if (threadId && itemId && delta) {
          handlers.onAgentMessageDelta?.({
            workspaceId: workspace_id,
            threadId,
            itemId,
            delta,
          });
        }
        return;
      }

      if (method === "turn/started") {
        const params = message.params as Record<string, unknown>;
        const turn = params.turn as Record<string, unknown> | undefined;
        const threadId = String(
          params.threadId ?? params.thread_id ?? turn?.threadId ?? turn?.thread_id ?? "",
        );
        const turnId = String(turn?.id ?? params.turnId ?? params.turn_id ?? "");
        if (threadId) {
          handlers.onTurnStarted?.(workspace_id, threadId, turnId);
        }
        return;
      }

      if (method === "error") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const turnId = String(params.turnId ?? params.turn_id ?? "");
        const error = (params.error as Record<string, unknown> | undefined) ?? {};
        const messageText = String(error.message ?? "");
        const willRetry = Boolean(params.willRetry ?? params.will_retry);
        if (threadId) {
          handlers.onTurnError?.(workspace_id, threadId, turnId, {
            message: messageText,
            willRetry,
          });
        }
        return;
      }

      if (method === "turn/completed") {
        const params = message.params as Record<string, unknown>;
        const turn = params.turn as Record<string, unknown> | undefined;
        const threadId = String(
          params.threadId ?? params.thread_id ?? turn?.threadId ?? turn?.thread_id ?? "",
        );
        const turnId = String(turn?.id ?? params.turnId ?? params.turn_id ?? "");
        if (threadId) {
          handlers.onTurnCompleted?.(workspace_id, threadId, turnId);
        }
        return;
      }

      if (method === "thread/compacted") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const turnId = String(params.turnId ?? params.turn_id ?? "");
        if (threadId && turnId) {
          handlers.onContextCompacted?.(workspace_id, threadId, turnId);
        }
        return;
      }

      if (method === "turn/permissionDenied") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const turnId = String(params.turnId ?? params.turn_id ?? "");
        const rawDenials =
          (params.permissionDenials as unknown[] | undefined) ??
          (params.permission_denials as unknown[] | undefined) ??
          [];
        if (threadId && rawDenials.length) {
          const denials = rawDenials
            .map((entry, index) => {
              if (!entry || typeof entry !== "object") {
                return null;
              }
              const record = entry as Record<string, unknown>;
              const toolName = String(
                record.toolName ?? record.tool_name ?? "",
              ).trim();
              if (!toolName) {
                return null;
              }
              const toolUseId =
                typeof record.toolUseId === "string"
                  ? record.toolUseId
                  : typeof record.tool_use_id === "string"
                    ? record.tool_use_id
                    : null;
              const toolInputValue =
                record.toolInput ?? record.tool_input ?? null;
              const toolInput =
                toolInputValue && typeof toolInputValue === "object"
                  ? (toolInputValue as Record<string, unknown>)
                  : null;
              const id = toolUseId || `${threadId}-${toolName}-${index}`;
              const denial: PermissionDenial = {
                id,
                workspace_id: workspace_id,
                thread_id: threadId,
                turn_id: turnId,
                tool_name: toolName,
                tool_use_id: toolUseId,
                tool_input: toolInput,
              };
              return denial;
            })
            .filter((item): item is PermissionDenial => Boolean(item));
          if (denials.length) {
            handlers.onPermissionDenied?.({
              workspaceId: workspace_id,
              threadId,
              turnId,
              denials,
            });
          }
        }
        return;
      }

      if (method === "turn/plan/updated") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const turnId = String(params.turnId ?? params.turn_id ?? "");
        if (threadId) {
          handlers.onTurnPlanUpdated?.(workspace_id, threadId, turnId, {
            explanation: params.explanation,
            plan: params.plan,
          });
        }
        return;
      }

      if (method === "turn/diff/updated") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const diff = String(params.diff ?? "");
        if (threadId && diff) {
          handlers.onTurnDiffUpdated?.(workspace_id, threadId, diff);
        }
        return;
      }

      if (method === "thread/created") {
        const params = message.params as Record<string, unknown>;
        const thread = params.thread as Record<string, unknown> | undefined;
        if (thread) {
          handlers.onThreadCreated?.(workspace_id, thread);
        }
        return;
      }

      if (method === "thread/tokenUsage/updated") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const tokenUsage =
          (params.tokenUsage as Record<string, unknown> | undefined) ??
          (params.token_usage as Record<string, unknown> | undefined);
        if (threadId && tokenUsage) {
          handlers.onThreadTokenUsageUpdated?.(workspace_id, threadId, tokenUsage);
        }
        return;
      }

      if (method === "item/completed") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const item = params.item as Record<string, unknown> | undefined;
        if (threadId && item) {
          handlers.onItemCompleted?.(workspace_id, threadId, item);
        }
        if (threadId && item?.type === "agentMessage") {
          const itemId = String(item.id ?? "");
          const text = String(item.text ?? "");
          if (itemId) {
            const payload: AgentCompleted = {
              workspaceId: workspace_id,
              threadId,
              itemId,
              text,
            };
            const modelValue = item.model ?? item.model_id ?? item.modelId;
            if (typeof modelValue === "string" && modelValue.trim()) {
              payload.model = modelValue;
            }
            handlers.onAgentMessageCompleted?.(payload);
          }
        }
        return;
      }

      if (method === "item/started") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const item = params.item as Record<string, unknown> | undefined;
        if (threadId && item) {
          handlers.onItemStarted?.(workspace_id, threadId, item);
          if (item.type === "agentMessage") {
            const itemId = String(item.id ?? "");
            const modelValue = item.model ?? item.model_id ?? item.modelId;
            if (itemId) {
              handlers.onAgentMessageStarted?.({
                workspaceId: workspace_id,
                threadId,
                itemId,
                model: typeof modelValue === "string" && modelValue.trim() ? modelValue : null,
              });
            }
          }
        }
        return;
      }

      if (method === "item/reasoning/summaryTextDelta") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const itemId = String(params.itemId ?? params.item_id ?? "");
        const delta = String(params.delta ?? "");
        if (threadId && itemId && delta) {
          handlers.onReasoningSummaryDelta?.(workspace_id, threadId, itemId, delta);
        }
        return;
      }

      if (method === "item/reasoning/summaryPartAdded") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const itemId = String(params.itemId ?? params.item_id ?? "");
        if (threadId && itemId) {
          handlers.onReasoningSummaryBoundary?.(workspace_id, threadId, itemId);
        }
        return;
      }

      if (method === "item/reasoning/textDelta") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const itemId = String(params.itemId ?? params.item_id ?? "");
        const delta = String(params.delta ?? "");
        if (threadId && itemId && delta) {
          handlers.onReasoningTextDelta?.(workspace_id, threadId, itemId, delta);
        }
        return;
      }

      if (method === "item/commandExecution/outputDelta") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const itemId = String(params.itemId ?? params.item_id ?? "");
        const delta = String(params.delta ?? "");
        if (threadId && itemId && delta) {
          handlers.onCommandOutputDelta?.(workspace_id, threadId, itemId, delta);
        }
        return;
      }

      if (method === "item/commandExecution/terminalInteraction") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const itemId = String(params.itemId ?? params.item_id ?? "");
        const stdin = String(params.stdin ?? "");
        if (threadId && itemId) {
          handlers.onTerminalInteraction?.(workspace_id, threadId, itemId, stdin);
        }
        return;
      }

      if (method === "item/fileChange/outputDelta") {
        const params = message.params as Record<string, unknown>;
        const threadId = String(params.threadId ?? params.thread_id ?? "");
        const itemId = String(params.itemId ?? params.item_id ?? "");
        const delta = String(params.delta ?? "");
        if (threadId && itemId && delta) {
          handlers.onFileChangeOutputDelta?.(workspace_id, threadId, itemId, delta);
        }
        return;
      }
    });

    return () => {
      unlisten();
    };
  }, [handlers]);
}
