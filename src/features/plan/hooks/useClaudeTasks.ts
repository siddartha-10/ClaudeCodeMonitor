import { useCallback, useEffect, useRef, useState } from "react";
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
    } catch (error) {
      // Silently fail - tasks may not exist for this session
      console.debug("Failed to fetch Claude tasks:", error);
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

  // Poll for task updates while processing
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
