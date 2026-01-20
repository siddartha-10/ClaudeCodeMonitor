import { useCallback, useState } from "react";
import type { WorkspaceInfo } from "../../../types";

type WorktreePromptState = {
  workspace: WorkspaceInfo;
  branch: string;
  isSubmitting: boolean;
  error: string | null;
} | null;

type UseWorktreePromptOptions = {
  addWorktreeAgent: (
    workspace: WorkspaceInfo,
    branch: string,
  ) => Promise<WorkspaceInfo | null>;
  connectWorkspace: (workspace: WorkspaceInfo) => Promise<void>;
  onSelectWorkspace: (workspaceId: string) => void;
  onCompactActivate?: () => void;
  onError?: (message: string) => void;
};

type UseWorktreePromptResult = {
  worktreePrompt: WorktreePromptState;
  openPrompt: (workspace: WorkspaceInfo) => void;
  confirmPrompt: () => Promise<void>;
  cancelPrompt: () => void;
  updateBranch: (value: string) => void;
};

export function useWorktreePrompt({
  addWorktreeAgent,
  connectWorkspace,
  onSelectWorkspace,
  onCompactActivate,
  onError,
}: UseWorktreePromptOptions): UseWorktreePromptResult {
  const [worktreePrompt, setWorktreePrompt] =
    useState<WorktreePromptState>(null);

  const openPrompt = useCallback((workspace: WorkspaceInfo) => {
    const defaultBranch = `claude/${new Date().toISOString().slice(0, 10)}-${Math.random()
      .toString(36)
      .slice(2, 6)}`;
    setWorktreePrompt({
      workspace,
      branch: defaultBranch,
      isSubmitting: false,
      error: null,
    });
  }, []);

  const updateBranch = useCallback((value: string) => {
    setWorktreePrompt((prev) =>
      prev ? { ...prev, branch: value, error: null } : prev,
    );
  }, []);

  const cancelPrompt = useCallback(() => {
    setWorktreePrompt(null);
  }, []);

  const confirmPrompt = useCallback(async () => {
    if (!worktreePrompt || worktreePrompt.isSubmitting) {
      return;
    }
    const { workspace, branch } = worktreePrompt;
    setWorktreePrompt((prev) =>
      prev ? { ...prev, isSubmitting: true, error: null } : prev,
    );
    try {
      const worktreeWorkspace = await addWorktreeAgent(workspace, branch);
      if (!worktreeWorkspace) {
        setWorktreePrompt(null);
        return;
      }
      onSelectWorkspace(worktreeWorkspace.id);
      if (!worktreeWorkspace.connected) {
        await connectWorkspace(worktreeWorkspace);
      }
      onCompactActivate?.();
      setWorktreePrompt(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setWorktreePrompt((prev) =>
        prev ? { ...prev, isSubmitting: false, error: message } : prev,
      );
      onError?.(message);
    }
  }, [
    addWorktreeAgent,
    connectWorkspace,
    onCompactActivate,
    onError,
    onSelectWorkspace,
    worktreePrompt,
  ]);

  return {
    worktreePrompt,
    openPrompt,
    confirmPrompt,
    cancelPrompt,
    updateBranch,
  };
}
