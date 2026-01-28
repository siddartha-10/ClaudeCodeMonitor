import { useCallback, useEffect, useState } from "react";
import { readGlobalClaudeMd, writeGlobalClaudeMd } from "../../../services/tauri";

export type GlobalClaudeMdState = {
  content: string;
  exists: boolean;
  truncated: boolean;
  isLoading: boolean;
  isSaving: boolean;
  error: string | null;
  isDirty: boolean;
};

export function useGlobalClaudeMd() {
  const [state, setState] = useState<GlobalClaudeMdState>({
    content: "",
    exists: false,
    truncated: false,
    isLoading: true,
    isSaving: false,
    error: null,
    isDirty: false,
  });

  const [originalContent, setOriginalContent] = useState("");

  const load = useCallback(async () => {
    setState((prev) => ({ ...prev, isLoading: true, error: null }));
    try {
      const response = await readGlobalClaudeMd();
      setState({
        content: response.content,
        exists: response.exists,
        truncated: response.truncated,
        isLoading: false,
        isSaving: false,
        error: null,
        isDirty: false,
      });
      setOriginalContent(response.content);
    } catch (err) {
      setState((prev) => ({
        ...prev,
        isLoading: false,
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const setContent = useCallback((content: string) => {
    setState((prev) => ({
      ...prev,
      content,
      isDirty: content !== originalContent,
    }));
  }, [originalContent]);

  const save = useCallback(async () => {
    setState((prev) => ({ ...prev, isSaving: true, error: null }));
    try {
      await writeGlobalClaudeMd(state.content);
      setOriginalContent(state.content);
      setState((prev) => ({
        ...prev,
        exists: true,
        isSaving: false,
        isDirty: false,
      }));
    } catch (err) {
      setState((prev) => ({
        ...prev,
        isSaving: false,
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, [state.content]);

  const refresh = useCallback(async () => {
    await load();
  }, [load]);

  return {
    ...state,
    setContent,
    save,
    refresh,
  };
}
