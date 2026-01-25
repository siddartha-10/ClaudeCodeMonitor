import type { ClaudeTask } from '../../../types';

interface Props {
  task: ClaudeTask;
  allTasks: ClaudeTask[];
  onClick?: () => void;
}

export function TaskItem({ task, allTasks, onClick }: Props) {
  const isBlocked = task.blockedBy.some(depId => {
    const dep = allTasks.find(t => t.id === depId);
    return dep && dep.status !== 'completed';
  });

  const statusColor = {
    pending: 'var(--status-warning)',
    in_progress: 'var(--border-accent)',
    completed: 'var(--status-success)',
  }[task.status];

  return (
    <div
      className={`task-item ${isBlocked ? 'blocked' : ''}`}
      onClick={onClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => e.key === 'Enter' && onClick?.()}
    >
      <span className="task-status-dot" style={{ backgroundColor: statusColor }} />
      <span className="task-id">#{task.id}</span>
      <span className="task-subject">{task.subject}</span>
      {task.owner && <span className="task-owner">@{task.owner}</span>}
      {isBlocked && (
        <span className="task-blocked-badge">
          blocked by {task.blockedBy.map(id => `#${id}`).join(', ')}
        </span>
      )}
    </div>
  );
}
