import type { ClaudeTask, ClaudeTaskStatus } from '../../types';

// Re-export for convenience
export type { ClaudeTask, ClaudeTaskStatus } from '../../types';

export interface TaskUpdate {
  subject?: string;
  description?: string;
  activeForm?: string;
  status?: ClaudeTaskStatus;
  owner?: string;
  addBlocks?: string[];
  addBlockedBy?: string[];
  metadata?: Record<string, unknown>;
}

export interface TaskListResponse {
  listId: string;
  tasks: ClaudeTask[];
}

export interface TaskListInfo {
  id: string;
  taskCount: number;
}
