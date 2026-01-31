import { useCallback, useEffect, useRef, useState } from "react";
import type { ThreadSummary, WorkspaceInfo } from "../../../types";
import { searchThread as searchThreadService } from "../../../services/tauri";

type SearchResult = {
  workspaceId: string;
  thread: ThreadSummary;
};

type UseThreadSearchOptions = {
  workspaces: WorkspaceInfo[];
  debounceMs?: number;
};

export function useThreadSearch({
  workspaces,
  debounceMs = 300,
}: UseThreadSearchOptions) {
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<SearchResult[] | null>(
    null,
  );
  const [isSearching, setIsSearching] = useState(false);
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (searchTimerRef.current) {
      clearTimeout(searchTimerRef.current);
    }
    let cancelled = false;
    const trimmed = searchQuery.trim();
    if (!trimmed) {
      setSearchResults(null);
      setIsSearching(false);
      return;
    }
    setIsSearching(true);
    searchTimerRef.current = setTimeout(async () => {
      try {
        // Search all workspaces in parallel for better performance
        const searchPromises = workspaces.map(async (workspace) => {
          try {
            const response = (await searchThreadService(
              workspace.id,
              trimmed,
            )) as Record<string, unknown>;
            const result = (response.result ?? response) as Record<
              string,
              unknown
            >;
            const data = Array.isArray(result?.data)
              ? (result.data as Record<string, unknown>[])
              : [];
            return data
              .map((thread) => {
                const id = String(thread?.id ?? "");
                const preview = String(thread?.preview ?? "").trim();
                const updatedAt = Number(
                  thread?.updatedAt ?? thread?.createdAt ?? 0,
                );
                return {
                  workspaceId: workspace.id,
                  thread: {
                    id,
                    name:
                      preview.length > 38
                        ? `${preview.slice(0, 38)}...`
                        : preview || id.slice(0, 12),
                    updatedAt,
                  },
                };
              })
              .filter((item) => item.thread.id);
          } catch {
            // workspace may not be connected, skip
            return [];
          }
        });

        const resultsArrays = await Promise.all(searchPromises);
        if (!cancelled) {
          const results = resultsArrays.flat();
          setSearchResults(results);
        }
      } catch (error) {
        console.error("[debug:sessions] search failed:", error);
        if (!cancelled) {
          setSearchResults([]);
        }
      } finally {
        if (!cancelled) {
          setIsSearching(false);
        }
      }
    }, debounceMs);
    return () => {
      cancelled = true;
      if (searchTimerRef.current) {
        clearTimeout(searchTimerRef.current);
      }
    };
  }, [searchQuery, workspaces, debounceMs]);

  const clearSearch = useCallback(() => {
    setSearchQuery("");
    searchInputRef.current?.focus();
  }, []);

  const blurSearch = useCallback(() => {
    searchInputRef.current?.blur();
  }, []);

  return {
    searchQuery,
    setSearchQuery,
    searchResults,
    isSearching,
    searchInputRef,
    clearSearch,
    blurSearch,
  };
}
