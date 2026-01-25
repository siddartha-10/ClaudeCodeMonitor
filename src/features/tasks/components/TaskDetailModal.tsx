import { useState, useCallback, useEffect } from 'react';
import type { ClaudeTask, ClaudeTaskStatus } from '../../../types';
import type { TaskUpdate } from '../types';

interface Props {
  task: ClaudeTask;
  allTasks: ClaudeTask[];
  onUpdate?: (updates: TaskUpdate) => Promise<void>;
  onDelete?: () => Promise<void>;
  onClose: () => void;
}

export function TaskDetailModal({ task, allTasks, onUpdate, onDelete, onClose }: Props) {
  const [editMode, setEditMode] = useState(false);
  const [subject, setSubject] = useState(task.subject);
  const [description, setDescription] = useState(task.description);
  const [status, setStatus] = useState<ClaudeTaskStatus>(task.status);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset form when task changes
  useEffect(() => {
    setSubject(task.subject);
    setDescription(task.description);
    setStatus(task.status);
    setEditMode(false);
    setError(null);
  }, [task]);

  const blockedByTasks = task.blockedBy
    .map(id => allTasks.find(t => t.id === id))
    .filter(Boolean) as ClaudeTask[];

  const blocksTasks = task.blocks
    .map(id => allTasks.find(t => t.id === id))
    .filter(Boolean) as ClaudeTask[];

  const isBlocked = blockedByTasks.some(t => t.status !== 'completed');

  const handleSave = useCallback(async () => {
    if (!onUpdate) return;
    if (!subject.trim()) {
      setError('Subject is required');
      return;
    }
    if (!description.trim()) {
      setError('Description is required');
      return;
    }

    setIsSubmitting(true);
    setError(null);

    try {
      const updates: TaskUpdate = {};
      if (subject !== task.subject) updates.subject = subject.trim();
      if (description !== task.description) updates.description = description.trim();
      if (status !== task.status) updates.status = status;

      if (Object.keys(updates).length > 0) {
        await onUpdate(updates);
      }
      setEditMode(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setIsSubmitting(false);
    }
  }, [task, subject, description, status, onUpdate]);

  const handleStatusChange = useCallback(async (newStatus: ClaudeTaskStatus) => {
    if (!onUpdate) return;
    if (newStatus === task.status) return;

    setIsSubmitting(true);
    setError(null);
    try {
      await onUpdate({ status: newStatus });
      setStatus(newStatus);
    } catch (err) {
      setError(String(err));
    } finally {
      setIsSubmitting(false);
    }
  }, [task.status, onUpdate]);

  const handleDelete = useCallback(async () => {
    if (!onDelete) return;
    if (!confirm('Are you sure you want to delete this task?')) return;

    setIsSubmitting(true);
    try {
      await onDelete();
      onClose();
    } catch (err) {
      setError(String(err));
      setIsSubmitting(false);
    }
  }, [onDelete, onClose]);

  // Close on Escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const statusLabel = {
    pending: 'Pending',
    in_progress: 'In Progress',
    completed: 'Completed',
  };

  const statusColor = {
    pending: 'var(--status-warning)',
    in_progress: 'var(--border-accent)',
    completed: 'var(--status-success)',
  };

  const canEdit = Boolean(onUpdate);

  return (
    <div className="task-modal-overlay" onClick={onClose}>
      <div className="task-modal" onClick={e => e.stopPropagation()}>
        <div className="task-modal-header">
          <span className="task-modal-id">#{task.id}</span>
          <button
            className="task-modal-close ghost icon-button"
            onClick={onClose}
            aria-label="Close"
          >
            <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
              <path d="M4.646 4.646a.5.5 0 0 1 .708 0L8 7.293l2.646-2.647a.5.5 0 0 1 .708.708L8.707 8l2.647 2.646a.5.5 0 0 1-.708.708L8 8.707l-2.646 2.647a.5.5 0 0 1-.708-.708L7.293 8 4.646 5.354a.5.5 0 0 1 0-.708z"/>
            </svg>
          </button>
        </div>

        <div className="task-modal-body">
          {editMode && canEdit ? (
            <>
              <div className="task-form-field">
                <label htmlFor="edit-subject">Subject</label>
                <input
                  id="edit-subject"
                  type="text"
                  value={subject}
                  onChange={(e) => setSubject(e.target.value)}
                  disabled={isSubmitting}
                />
              </div>
              <div className="task-form-field">
                <label htmlFor="edit-description">Description</label>
                <textarea
                  id="edit-description"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  rows={6}
                  disabled={isSubmitting}
                />
              </div>
            </>
          ) : (
            <>
              <h2 className="task-modal-subject">{task.subject}</h2>
              <p className="task-modal-description">{task.description}</p>
            </>
          )}

          <div className="task-modal-meta">
            <div className="task-meta-row">
              <span className="task-meta-label">Status</span>
              {editMode && canEdit ? (
                <select
                  value={status}
                  onChange={(e) => setStatus(e.target.value as ClaudeTaskStatus)}
                  disabled={isSubmitting}
                  className="task-status-select"
                >
                  <option value="pending">Pending</option>
                  <option value="in_progress">In Progress</option>
                  <option value="completed">Completed</option>
                </select>
              ) : (
                <div className="task-status-badge-row">
                  <span
                    className="task-status-badge"
                    style={{
                      backgroundColor: statusColor[task.status],
                      color: task.status === 'pending' ? 'var(--text-strong)' : 'white'
                    }}
                  >
                    {statusLabel[task.status]}
                  </span>
                  {canEdit && !editMode && task.status !== 'completed' && (
                    <div className="task-quick-status">
                      {task.status === 'pending' && (
                        <button
                          className="ghost"
                          onClick={() => handleStatusChange('in_progress')}
                          disabled={isSubmitting || isBlocked}
                          title={isBlocked ? 'Task is blocked' : 'Start working'}
                        >
                          Start
                        </button>
                      )}
                      {task.status === 'in_progress' && (
                        <button
                          className="ghost"
                          onClick={() => handleStatusChange('completed')}
                          disabled={isSubmitting}
                        >
                          Complete
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>

            {task.owner && (
              <div className="task-meta-row">
                <span className="task-meta-label">Owner</span>
                <span className="task-meta-value">@{task.owner}</span>
              </div>
            )}

            {task.activeForm && (
              <div className="task-meta-row">
                <span className="task-meta-label">Active Form</span>
                <span className="task-meta-value">{task.activeForm}</span>
              </div>
            )}

            {blockedByTasks.length > 0 && (
              <div className="task-meta-row task-meta-deps">
                <span className="task-meta-label">Blocked By</span>
                <div className="task-dep-list">
                  {blockedByTasks.map(dep => (
                    <span
                      key={dep.id}
                      className={`task-dep-item ${dep.status === 'completed' ? 'resolved' : 'blocking'}`}
                    >
                      #{dep.id} {dep.subject}
                    </span>
                  ))}
                </div>
              </div>
            )}

            {blocksTasks.length > 0 && (
              <div className="task-meta-row task-meta-deps">
                <span className="task-meta-label">Blocks</span>
                <div className="task-dep-list">
                  {blocksTasks.map(blocked => (
                    <span key={blocked.id} className="task-dep-item">
                      #{blocked.id} {blocked.subject}
                    </span>
                  ))}
                </div>
              </div>
            )}
          </div>

          {error && (
            <div className="task-form-error">{error}</div>
          )}
        </div>

        <div className="task-modal-footer">
          {editMode && canEdit ? (
            <>
              <button
                className="ghost"
                onClick={() => {
                  setEditMode(false);
                  setSubject(task.subject);
                  setDescription(task.description);
                  setStatus(task.status);
                  setError(null);
                }}
                disabled={isSubmitting}
              >
                Cancel
              </button>
              <button
                className="primary"
                onClick={handleSave}
                disabled={isSubmitting}
              >
                {isSubmitting ? 'Saving...' : 'Save Changes'}
              </button>
            </>
          ) : (
            <>
              {onDelete && (
                <button
                  className="ghost task-delete-button"
                  onClick={handleDelete}
                  disabled={isSubmitting}
                >
                  Delete
                </button>
              )}
              {canEdit && (
                <button
                  className="secondary"
                  onClick={() => setEditMode(true)}
                  disabled={isSubmitting}
                >
                  Edit
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
