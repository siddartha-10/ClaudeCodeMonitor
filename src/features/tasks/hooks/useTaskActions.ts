import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ClaudeTask, TaskUpdate } from '../types';

export function useTaskActions(listId: string | null) {
  const createTask = useCallback(async (subject: string, description: string, activeForm?: string) => {
    if (!listId) throw new Error('No list ID');
    return invoke<ClaudeTask>('task_create', { listId, subject, description, activeForm });
  }, [listId]);

  const updateTask = useCallback(async (taskId: string, updates: TaskUpdate) => {
    if (!listId) throw new Error('No list ID');
    return invoke<ClaudeTask>('task_update', { listId, taskId, updates });
  }, [listId]);

  const deleteTask = useCallback(async (taskId: string) => {
    if (!listId) throw new Error('No list ID');
    return invoke<void>('task_delete', { listId, taskId });
  }, [listId]);

  return { createTask, updateTask, deleteTask };
}
