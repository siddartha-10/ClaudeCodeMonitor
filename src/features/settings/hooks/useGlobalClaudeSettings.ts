import { useCallback, useMemo } from "react";
import { fileRead, fileWrite } from "../../../services/tauri";
import { useFileEditor } from "../../shared/hooks/useFileEditor";

// Use a constant key since global settings is not workspace-specific
const GLOBAL_SETTINGS_KEY = "global-settings";

export function useGlobalClaudeSettings() {
  const read = useCallback(async () => {
    return fileRead("global", "settings");
  }, []);

  const write = useCallback(async (content: string) => {
    return fileWrite("global", "settings", content);
  }, []);

  const editor = useFileEditor({
    key: GLOBAL_SETTINGS_KEY,
    read,
    write,
    readErrorTitle: "Failed to read global settings",
    writeErrorTitle: "Failed to save global settings",
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
