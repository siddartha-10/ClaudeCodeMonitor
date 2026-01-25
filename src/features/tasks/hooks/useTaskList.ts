import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ClaudeTask, TaskListResponse } from '../types';

export function useTaskList(listId: string | null) {
  const [tasks, setTasks] = useState<ClaudeTask[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchTasks = useCallback(async () => {
    if (!listId) return;
    setLoading(true);
    try {
      const result = await invoke<TaskListResponse>('task_list_read', { listId });
      setTasks(result.tasks);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [listId]);

  // Initial fetch
  useEffect(() => { fetchTasks(); }, [fetchTasks]);

  // Listen for real-time updates
  useEffect(() => {
    if (!listId) return;

    // Start the watcher first
    invoke('task_watcher_start', { listId }).catch(() => {
      // Silently fail - watcher is optional enhancement
    });

    const unlisten = listen(`task-list-changed:${listId}`, () => fetchTasks());

    return () => {
      unlisten.then(fn => fn());
      invoke('task_watcher_stop', { listId }).catch(() => {});
    };
  }, [listId, fetchTasks]);

  // Computed properties
  const pendingTasks = tasks.filter(t => t.status === 'pending');
  const inProgressTasks = tasks.filter(t => t.status === 'in_progress');
  const completedTasks = tasks.filter(t => t.status === 'completed');

  const availableTasks = tasks.filter(t =>
    t.status === 'pending' &&
    t.blockedBy.every(depId => tasks.find(d => d.id === depId)?.status === 'completed')
  );

  return { tasks, loading, error, refresh: fetchTasks, pendingTasks, inProgressTasks, completedTasks, availableTasks };
}
