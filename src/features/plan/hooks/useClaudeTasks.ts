import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { ClaudeTask, TurnPlan, TurnPlanStep, TurnPlanStepStatus } from "../../../types";
import { getClaudeTasks } from "../../../services/tauri";

const POLL_INTERVAL_PROCESSING_MS = 5000;
const POLL_INTERVAL_IDLE_MS = 15000;

type UseClaudeTasksOptions = {
  activeThreadId: string | null;
  isProcessing: boolean;
};

type UseClaudeTasksResult = {
  tasks: ClaudeTask[];
  plan: TurnPlan | null;
};

function taskStatusToStepStatus(status: string): TurnPlanStepStatus {
  if (status === "completed" || status === "done") {
    return "completed";
  }
  if (status === "in_progress" || status === "inProgress") {
    return "inProgress";
  }
  return "pending";
}

function tasksToTurnPlan(tasks: ClaudeTask[], threadId: string): TurnPlan | null {
  if (tasks.length === 0) {
    return null;
  }

  const steps: TurnPlanStep[] = tasks.map((task) => ({
    step: task.subject,
    status: taskStatusToStepStatus(task.status),
  }));

  return {
    turnId: threadId,
    explanation: null,
    steps,
  };
}

export function useClaudeTasks({
  activeThreadId,
  isProcessing,
}: UseClaudeTasksOptions): UseClaudeTasksResult {
  const [tasks, setTasks] = useState<ClaudeTask[]>([]);
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const lastThreadIdRef = useRef<string | null>(null);

  const fetchTasks = useCallback(async (threadId: string) => {
    try {
      const response = await getClaudeTasks(threadId);
      setTasks(response.tasks);
    } catch {
      // Silently fail - tasks may not exist for this session
    }
  }, []);

  // Clear tasks when thread changes
  useEffect(() => {
    if (activeThreadId !== lastThreadIdRef.current) {
      setTasks([]);
      lastThreadIdRef.current = activeThreadId;
    }
  }, [activeThreadId]);

  // Fetch tasks on mount and when thread changes
  useEffect(() => {
    if (!activeThreadId) {
      setTasks([]);
      return;
    }

    fetchTasks(activeThreadId);
  }, [activeThreadId, fetchTasks]);

  // Set up file watcher for real-time updates
  useEffect(() => {
    if (!activeThreadId) {
      return;
    }

    let unlistenFn: (() => void) | null = null;
    let cancelled = false;

    // Start the file watcher
    invoke("task_watcher_start", { listId: activeThreadId })
      .then(() => {
        if (cancelled) return;
        // Listen for task list changes
        listen(`task-list-changed:${activeThreadId}`, () => {
          fetchTasks(activeThreadId);
        }).then((unlisten) => {
          if (cancelled) {
            unlisten();
          } else {
            unlistenFn = unlisten;
          }
        });
      })
      .catch(() => {
        // Silently fail - watcher is optional enhancement
      });

    return () => {
      cancelled = true;
      if (unlistenFn) {
        unlistenFn();
      }
      invoke("task_watcher_stop", { listId: activeThreadId }).catch(() => {
        // Ignore errors on cleanup
      });
    };
  }, [activeThreadId, fetchTasks]);

  // Poll for task updates as fallback (less frequent with file watcher)
  useEffect(() => {
    if (!activeThreadId) {
      return;
    }

    const interval = isProcessing ? POLL_INTERVAL_PROCESSING_MS : POLL_INTERVAL_IDLE_MS;

    pollIntervalRef.current = setInterval(() => {
      fetchTasks(activeThreadId);
    }, interval);

    return () => {
      if (pollIntervalRef.current) {
        clearInterval(pollIntervalRef.current);
        pollIntervalRef.current = null;
      }
    };
  }, [activeThreadId, isProcessing, fetchTasks]);

  const plan = activeThreadId ? tasksToTurnPlan(tasks, activeThreadId) : null;

  return {
    tasks,
    plan,
  };
}
