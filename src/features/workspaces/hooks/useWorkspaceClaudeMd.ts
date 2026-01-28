import { useCallback, useEffect, useRef, useState } from "react";
import type { WorkspaceInfo } from "../../../types";
import { readClaudeMd, writeClaudeMd } from "../../../services/tauri";

type UseWorkspaceClaudeMdOptions = {
  activeWorkspace: WorkspaceInfo | null;
};

export function useWorkspaceClaudeMd({
  activeWorkspace,
}: UseWorkspaceClaudeMdOptions) {
  const [content, setContent] = useState("");
  const [originalContent, setOriginalContent] = useState("");
  const [exists, setExists] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const lastFetchedWorkspaceId = useRef<string | null>(null);
  const inFlight = useRef<string | null>(null);

  const workspaceId = activeWorkspace?.id ?? null;
  const isConnected = Boolean(activeWorkspace?.connected);
  const isDirty = content !== originalContent;

  const refresh = useCallback(async () => {
    if (!workspaceId || !isConnected) {
      return;
    }
    if (inFlight.current === workspaceId) {
      return;
    }
    inFlight.current = workspaceId;
    const requestWorkspaceId = workspaceId;
    setIsLoading(true);
    setError(null);

    try {
      const response = await readClaudeMd(requestWorkspaceId);
      if (requestWorkspaceId === workspaceId) {
        setExists(response.exists);
        setContent(response.content);
        setOriginalContent(response.content);
        setTruncated(response.truncated);
        lastFetchedWorkspaceId.current = requestWorkspaceId;
      }
    } catch (err) {
      if (requestWorkspaceId === workspaceId) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (inFlight.current === requestWorkspaceId) {
        inFlight.current = null;
        setIsLoading(false);
      }
    }
  }, [isConnected, workspaceId]);

  const save = useCallback(async () => {
    if (!workspaceId || !isConnected) {
      return;
    }
    if (isSaving) {
      return;
    }

    setIsSaving(true);
    setError(null);

    try {
      await writeClaudeMd(workspaceId, content);
      setOriginalContent(content);
      setExists(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsSaving(false);
    }
  }, [content, isConnected, isSaving, workspaceId]);

  // Reset state when workspace changes
  useEffect(() => {
    setContent("");
    setOriginalContent("");
    setExists(false);
    setTruncated(false);
    setError(null);
    lastFetchedWorkspaceId.current = null;
    inFlight.current = null;
    setIsLoading(Boolean(workspaceId && isConnected));
  }, [isConnected, workspaceId]);

  // Auto-load when workspace changes
  useEffect(() => {
    if (!workspaceId || !isConnected) {
      return;
    }
    if (lastFetchedWorkspaceId.current === workspaceId) {
      return;
    }
    refresh();
  }, [isConnected, refresh, workspaceId]);

  return {
    content,
    exists,
    truncated,
    isLoading,
    isSaving,
    error,
    isDirty,
    setContent,
    refresh,
    save,
  };
}
