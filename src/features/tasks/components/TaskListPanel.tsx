import type { ClaudeTask } from '../../../types';
import { TaskItem } from './TaskItem';
import './TaskListPanel.css';

interface Props {
  tasks: ClaudeTask[];
  onTaskClick?: (task: ClaudeTask) => void;
  onCreateClick?: () => void;
  loading?: boolean;
  error?: string | null;
}

export function TaskListPanel({ tasks, onTaskClick, onCreateClick, loading, error }: Props) {
  const pending = tasks.filter(t => t.status === 'pending');
  const inProgress = tasks.filter(t => t.status === 'in_progress');
  const completed = tasks.filter(t => t.status === 'completed');

  const totalCount = tasks.length;
  const completedCount = completed.length;

  return (
    <div className="task-list-panel">
      <div className="task-list-header">
        <div className="task-list-title">
          <span>Tasks</span>
          {totalCount > 0 && (
            <span className="task-list-progress">{completedCount}/{totalCount}</span>
          )}
        </div>
        {onCreateClick && (
          <button className="task-add-button ghost" onClick={onCreateClick}>
            + Add
          </button>
        )}
      </div>

      {error && (
        <div className="task-list-error">{error}</div>
      )}

      {loading && tasks.length === 0 && (
        <div className="task-list-empty">Loading tasks...</div>
      )}

      {!loading && tasks.length === 0 && !error && (
        <div className="task-list-empty">No tasks yet.</div>
      )}

      {inProgress.length > 0 && (
        <section className="task-section">
          <h4 className="task-section-header">In Progress ({inProgress.length})</h4>
          {inProgress.map(task => (
            <TaskItem
              key={task.id}
              task={task}
              allTasks={tasks}
              onClick={() => onTaskClick?.(task)}
            />
          ))}
        </section>
      )}

      {pending.length > 0 && (
        <section className="task-section">
          <h4 className="task-section-header">Pending ({pending.length})</h4>
          {pending.map(task => (
            <TaskItem
              key={task.id}
              task={task}
              allTasks={tasks}
              onClick={() => onTaskClick?.(task)}
            />
          ))}
        </section>
      )}

      {completed.length > 0 && (
        <section className="task-section">
          <h4 className="task-section-header">Completed ({completed.length})</h4>
          {completed.map(task => (
            <TaskItem
              key={task.id}
              task={task}
              allTasks={tasks}
              onClick={() => onTaskClick?.(task)}
            />
          ))}
        </section>
      )}
    </div>
  );
}
