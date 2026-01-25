import { useState, useCallback } from 'react';

interface Props {
  onSubmit: (subject: string, description: string, activeForm?: string) => Promise<void>;
  onCancel?: () => void;
  isSubmitting?: boolean;
}

export function TaskCreateForm({ onSubmit, onCancel, isSubmitting }: Props) {
  const [subject, setSubject] = useState('');
  const [description, setDescription] = useState('');
  const [activeForm, setActiveForm] = useState('');
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = useCallback(async (e: React.FormEvent) => {
    e.preventDefault();

    if (!subject.trim()) {
      setError('Subject is required');
      return;
    }

    if (!description.trim()) {
      setError('Description is required');
      return;
    }

    try {
      setError(null);
      await onSubmit(subject.trim(), description.trim(), activeForm.trim() || undefined);
      // Reset form on success
      setSubject('');
      setDescription('');
      setActiveForm('');
    } catch (err) {
      setError(String(err));
    }
  }, [subject, description, activeForm, onSubmit]);

  return (
    <form className="task-create-form" onSubmit={handleSubmit}>
      <div className="task-form-field">
        <label htmlFor="task-subject">Subject</label>
        <input
          id="task-subject"
          type="text"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
          placeholder="Brief task title..."
          disabled={isSubmitting}
          autoFocus
        />
      </div>

      <div className="task-form-field">
        <label htmlFor="task-description">Description</label>
        <textarea
          id="task-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Detailed description of what needs to be done..."
          rows={4}
          disabled={isSubmitting}
        />
      </div>

      <div className="task-form-field">
        <label htmlFor="task-active-form">Active Form (optional)</label>
        <input
          id="task-active-form"
          type="text"
          value={activeForm}
          onChange={(e) => setActiveForm(e.target.value)}
          placeholder="e.g., 'Implementing feature...'"
          disabled={isSubmitting}
        />
        <span className="task-form-hint">
          Shown in spinner when task is in progress
        </span>
      </div>

      {error && (
        <div className="task-form-error">{error}</div>
      )}

      <div className="task-form-actions">
        {onCancel && (
          <button
            type="button"
            className="ghost"
            onClick={onCancel}
            disabled={isSubmitting}
          >
            Cancel
          </button>
        )}
        <button
          type="submit"
          className="primary"
          disabled={isSubmitting || !subject.trim() || !description.trim()}
        >
          {isSubmitting ? 'Creating...' : 'Create Task'}
        </button>
      </div>
    </form>
  );
}
