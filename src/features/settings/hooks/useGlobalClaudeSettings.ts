import { useCallback, useEffect, useRef, useState } from "react";
import {
  readGlobalClaudeSettings,
  writeGlobalClaudeSettings,
} from "../../../services/tauri";

export function useGlobalClaudeSettings() {
  const [content, setContent] = useState("");
  const [originalContent, setOriginalContent] = useState("");
  const [exists, setExists] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const hasFetched = useRef(false);
  const inFlight = useRef(false);

  const isDirty = content !== originalContent;

  const refresh = useCallback(async () => {
    if (inFlight.current) {
      return;
    }
    inFlight.current = true;
    setIsLoading(true);
    setError(null);

    try {
      const response = await readGlobalClaudeSettings();
      setExists(response.exists);
      setContent(response.content);
      setOriginalContent(response.content);
      setTruncated(response.truncated);
      hasFetched.current = true;
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      inFlight.current = false;
      setIsLoading(false);
    }
  }, []);

  const save = useCallback(async () => {
    if (isSaving) {
      return;
    }

    setIsSaving(true);
    setError(null);

    try {
      await writeGlobalClaudeSettings(content);
      setOriginalContent(content);
      setExists(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsSaving(false);
    }
  }, [content, isSaving]);

  // Auto-load on mount
  useEffect(() => {
    if (hasFetched.current) {
      return;
    }
    refresh();
  }, [refresh]);

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
