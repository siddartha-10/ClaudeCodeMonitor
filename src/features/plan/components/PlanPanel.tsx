import { useState } from "react";
import type { ClaudeTask, TurnPlan } from "../../../types";
import { TaskListPanel } from "../../tasks/components/TaskListPanel";
import { TaskDetailModal } from "../../tasks/components/TaskDetailModal";

type PlanPanelProps = {
  plan: TurnPlan | null;
  tasks?: ClaudeTask[];
  isProcessing: boolean;
};

function formatProgress(plan: TurnPlan) {
  const total = plan.steps.length;
  if (!total) {
    return "";
  }
  const completed = plan.steps.filter((step) => step.status === "completed").length;
  return `${completed}/${total}`;
}

function statusLabel(status: TurnPlan["steps"][number]["status"]) {
  if (status === "completed") {
    return "[x]";
  }
  if (status === "inProgress") {
    return "[>]";
  }
  return "[ ]";
}

export function PlanPanel({ plan, tasks, isProcessing }: PlanPanelProps) {
  const [selectedTask, setSelectedTask] = useState<ClaudeTask | null>(null);

  // If we have full task data, use the new TaskListPanel
  if (tasks && tasks.length > 0) {
    return (
      <>
        <TaskListPanel
          tasks={tasks}
          onTaskClick={setSelectedTask}
        />
        {selectedTask && (
          <TaskDetailModal
            task={selectedTask}
            allTasks={tasks}
            onClose={() => setSelectedTask(null)}
          />
        )}
      </>
    );
  }

  // Fall back to legacy TurnPlan display
  const progress = plan ? formatProgress(plan) : "";
  const steps = plan?.steps ?? [];
  const showEmpty = !steps.length && !plan?.explanation;
  const emptyLabel = isProcessing ? "Waiting on a plan..." : "No active plan.";

  return (
    <aside className="plan-panel">
      <div className="plan-header">
        <span>Plan</span>
        {progress && <span className="plan-progress">{progress}</span>}
      </div>
      {plan?.explanation && (
        <div className="plan-explanation">{plan.explanation}</div>
      )}
      {showEmpty ? (
        <div className="plan-empty">{emptyLabel}</div>
      ) : (
        <ol className="plan-list">
          {steps.map((step, index) => (
            <li key={`${step.step}-${index}`} className={`plan-step ${step.status}`}>
              <span className="plan-step-status" aria-hidden>
                {statusLabel(step.status)}
              </span>
              <span className="plan-step-text">{step.step}</span>
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}
