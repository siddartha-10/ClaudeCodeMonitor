import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, MouseEvent } from "react";

import type { ThreadSummary } from "../../../types";

type ThreadStatusMap = Record<
  string,
  { isProcessing: boolean; hasUnread: boolean; isReviewing: boolean }
>;

type ThreadRow = {
  thread: ThreadSummary;
  depth: number;
};

type ThreadListProps = {
  workspaceId: string;
  pinnedRows: ThreadRow[];
  unpinnedRows: ThreadRow[];
  totalThreadRoots: number;
  isExpanded: boolean;
  nextCursor: string | null;
  isPaging: boolean;
  nested?: boolean;
  showLoadOlder?: boolean;
  activeWorkspaceId: string | null;
  activeThreadId: string | null;
  threadStatusById: ThreadStatusMap;
  getThreadTime: (thread: ThreadSummary) => string | null;
  isThreadPinned: (workspaceId: string, threadId: string) => boolean;
  onToggleExpanded: (workspaceId: string) => void;
  onLoadOlderThreads: (workspaceId: string) => void;
  onSelectThread: (workspaceId: string, threadId: string) => void;
  onShowThreadMenu: (
    event: MouseEvent,
    workspaceId: string,
    threadId: string,
    canPin: boolean,
  ) => void;
};

export function ThreadList({
  workspaceId,
  pinnedRows,
  unpinnedRows,
  totalThreadRoots,
  isExpanded,
  nextCursor,
  isPaging,
  nested,
  showLoadOlder = true,
  activeWorkspaceId,
  activeThreadId,
  threadStatusById,
  getThreadTime,
  isThreadPinned,
  onToggleExpanded,
  onLoadOlderThreads,
  onSelectThread,
  onShowThreadMenu,
}: ThreadListProps) {
  const [collapsedThreadIds, setCollapsedThreadIds] = useState<Set<string>>(
    () => new Set(),
  );
  const knownParentIdsRef = useRef<Set<string>>(new Set());
  const parentIds = useMemo(() => {
    const ids = new Set<string>();
    const collectParents = (rows: ThreadRow[]) => {
      rows.forEach((row, index) => {
        if (rows[index + 1]?.depth > row.depth) {
          ids.add(row.thread.id);
        }
      });
    };
    collectParents(pinnedRows);
    collectParents(unpinnedRows);
    return ids;
  }, [pinnedRows, unpinnedRows]);

  useEffect(() => {
    setCollapsedThreadIds((prev) => {
      let changed = false;
      const next = new Set(prev);
      const knownParentIds = knownParentIdsRef.current;

      prev.forEach((id) => {
        if (!parentIds.has(id)) {
          next.delete(id);
          changed = true;
        }
      });

      parentIds.forEach((id) => {
        if (!knownParentIds.has(id)) {
          next.add(id);
          changed = true;
        }
      });

      knownParentIdsRef.current = new Set(parentIds);
      return changed ? next : prev;
    });
  }, [parentIds]);

  const toggleThreadCollapse = useCallback((threadId: string) => {
    setCollapsedThreadIds((prev) => {
      const next = new Set(prev);
      if (next.has(threadId)) {
        next.delete(threadId);
      } else {
        next.add(threadId);
      }
      return next;
    });
  }, []);

  const indentUnit = nested ? 10 : 14;
  const buildVisibleRows = useCallback((rows: ThreadRow[]) => {
    const visibleRows: Array<{
      row: ThreadRow;
      hasChildren: boolean;
      isCollapsed: boolean;
    }> = [];
    let hiddenDepth: number | null = null;

    rows.forEach((row, index) => {
      if (hiddenDepth !== null && row.depth > hiddenDepth) {
        return;
      }
      if (hiddenDepth !== null && row.depth <= hiddenDepth) {
        hiddenDepth = null;
      }
      const hasChildren = rows[index + 1]?.depth > row.depth;
      const isCollapsed = hasChildren && collapsedThreadIds.has(row.thread.id);
      if (isCollapsed) {
        hiddenDepth = row.depth;
      }
      visibleRows.push({ row, hasChildren, isCollapsed });
    });

    return visibleRows;
  }, [collapsedThreadIds]);

  const visiblePinnedRows = useMemo(
    () => buildVisibleRows(pinnedRows),
    [pinnedRows, buildVisibleRows],
  );
  const visibleUnpinnedRows = useMemo(
    () => buildVisibleRows(unpinnedRows),
    [unpinnedRows, buildVisibleRows],
  );

  const renderThreadRow = ({
    row: { thread, depth },
    hasChildren,
    isCollapsed,
  }: {
    row: ThreadRow;
    hasChildren: boolean;
    isCollapsed: boolean;
  }) => {
    const relativeTime = getThreadTime(thread);
    const indentStyle =
      depth > 0
        ? ({ "--thread-indent": `${depth * indentUnit}px` } as CSSProperties)
        : undefined;
    const status = threadStatusById[thread.id];
    const statusClass = status?.isReviewing
      ? "reviewing"
      : status?.isProcessing
        ? "processing"
        : status?.hasUnread
          ? "unread"
          : "ready";
    const canPin = depth === 0;
    const isPinned = canPin && isThreadPinned(workspaceId, thread.id);

    return (
      <div
        key={thread.id}
        className={`thread-row ${
          workspaceId === activeWorkspaceId && thread.id === activeThreadId
            ? "active"
            : ""
        }`}
        style={indentStyle}
        onClick={() => onSelectThread(workspaceId, thread.id)}
        onContextMenu={(event) =>
          onShowThreadMenu(event, workspaceId, thread.id, canPin)
        }
        role="button"
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onSelectThread(workspaceId, thread.id);
          }
        }}
      >
        {hasChildren ? (
          <button
            type="button"
            className={`thread-toggle${isCollapsed ? "" : " expanded"}`}
            aria-label={isCollapsed ? "Expand thread" : "Collapse thread"}
            onClick={(event) => {
              event.stopPropagation();
              toggleThreadCollapse(thread.id);
            }}
          >
            <span className="thread-toggle-icon" aria-hidden>
              ▸
            </span>
          </button>
        ) : (
          <span className="thread-toggle-spacer" aria-hidden />
        )}
        <span className={`thread-status ${statusClass}`} aria-hidden />
        {isPinned && <span className="thread-pin-icon" aria-label="Pinned">📌</span>}
        <span className="thread-name">{thread.name}</span>
        <div className="thread-meta">
          {relativeTime && <span className="thread-time">{relativeTime}</span>}
          <div className="thread-menu">
            <div className="thread-menu-trigger" aria-hidden="true" />
          </div>
        </div>
      </div>
    );
  };

  return (
    <div className={`thread-list${nested ? " thread-list-nested" : ""}`}>
      {visiblePinnedRows.map((row) => renderThreadRow(row))}
      {visiblePinnedRows.length > 0 && visibleUnpinnedRows.length > 0 && (
        <div className="thread-list-separator" aria-hidden="true" />
      )}
      {visibleUnpinnedRows.map((row) => renderThreadRow(row))}
      {totalThreadRoots > 3 && (
        <button
          className="thread-more"
          onClick={(event) => {
            event.stopPropagation();
            onToggleExpanded(workspaceId);
          }}
        >
          {isExpanded ? "Show less" : "More..."}
        </button>
      )}
      {showLoadOlder && nextCursor && (isExpanded || totalThreadRoots <= 3) && (
        <button
          className="thread-more"
          onClick={(event) => {
            event.stopPropagation();
            onLoadOlderThreads(workspaceId);
          }}
          disabled={isPaging}
        >
          {isPaging
            ? "Loading..."
            : totalThreadRoots === 0
              ? "Search older..."
              : "Load older..."}
        </button>
      )}
    </div>
  );
}
