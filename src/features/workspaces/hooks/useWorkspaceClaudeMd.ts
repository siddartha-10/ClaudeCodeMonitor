import { useCallback, useMemo } from "react";
import type { WorkspaceInfo } from "../../../types";
import { fileRead, fileWrite } from "../../../services/tauri";
import { useFileEditor } from "../../shared/hooks/useFileEditor";

type UseWorkspaceClaudeMdOptions = {
  activeWorkspace: WorkspaceInfo | null;
};

export function useWorkspaceClaudeMd({
  activeWorkspace,
}: UseWorkspaceClaudeMdOptions) {
  const workspaceId = activeWorkspace?.id ?? null;
  const isConnected = Boolean(activeWorkspace?.connected);

  // Only enable the editor when we have a connected workspace
  const editorKey = workspaceId && isConnected ? workspaceId : null;

  const read = useCallback(async () => {
    if (!workspaceId) {
      throw new Error("No workspace selected");
    }
    return fileRead("workspace", "claude_md", workspaceId);
  }, [workspaceId]);

  const write = useCallback(
    async (content: string) => {
      if (!workspaceId) {
        throw new Error("No workspace selected");
      }
      return fileWrite("workspace", "claude_md", content, workspaceId);
    },
    [workspaceId],
  );

  const editor = useFileEditor({
    key: editorKey,
    read,
    write,
    readErrorTitle: "Failed to read CLAUDE.md",
    writeErrorTitle: "Failed to save CLAUDE.md",
  });

  // Memoize the return value to maintain stable reference
  return useMemo(
    () => ({
      content: editor.content,
      exists: editor.exists,
      truncated: editor.truncated,
      isLoading: editor.isLoading,
      isSaving: editor.isSaving,
      error: editor.error,
      isDirty: editor.isDirty,
      setContent: editor.setContent,
      refresh: editor.refresh,
      save: editor.save,
    }),
    [
      editor.content,
      editor.exists,
      editor.truncated,
      editor.isLoading,
      editor.isSaving,
      editor.error,
      editor.isDirty,
      editor.setContent,
      editor.refresh,
      editor.save,
    ],
  );
}
