import type {
  AccountSnapshot,
  RateLimitSnapshot,
  ThreadSummary,
  WorkspaceInfo,
} from "../../../types";
import { createPortal } from "react-dom";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { RefObject } from "react";
import { FolderOpen, Search } from "lucide-react";
import X from "lucide-react/dist/esm/icons/x";
import { searchThread as searchThreadService } from "../../../services/tauri";
import { SidebarCornerActions } from "./SidebarCornerActions";
import { SidebarFooter } from "./SidebarFooter";
import { SidebarHeader } from "./SidebarHeader";
import { ThreadList } from "./ThreadList";
import { ThreadLoading } from "./ThreadLoading";
import { WorktreeSection } from "./WorktreeSection";
import { PinnedThreadList } from "./PinnedThreadList";
import { WorkspaceCard } from "./WorkspaceCard";
import { WorkspaceGroup } from "./WorkspaceGroup";
import { useCollapsedGroups } from "../hooks/useCollapsedGroups";
import { useSidebarMenus } from "../hooks/useSidebarMenus";
import { useSidebarScrollFade } from "../hooks/useSidebarScrollFade";
import { useThreadRows } from "../hooks/useThreadRows";
import { getUsageLabels } from "../utils/usageLabels";
import { formatRelativeTimeShort } from "../../../utils/time";

const COLLAPSED_GROUPS_STORAGE_KEY = "claude-code-monitor.collapsedGroups";
const UNGROUPED_COLLAPSE_ID = "__ungrouped__";
const ADD_MENU_WIDTH = 200;

type WorkspaceGroupSection = {
  id: string | null;
  name: string;
  workspaces: WorkspaceInfo[];
};

type SidebarProps = {
  workspaces: WorkspaceInfo[];
  groupedWorkspaces: WorkspaceGroupSection[];
  hasWorkspaceGroups: boolean;
  deletingWorktreeIds: Set<string>;
  threadsByWorkspace: Record<string, ThreadSummary[]>;
  threadParentById: Record<string, string>;
  threadStatusById: Record<
    string,
    { isProcessing: boolean; hasUnread: boolean; isReviewing: boolean }
  >;
  threadListLoadingByWorkspace: Record<string, boolean>;
  threadListPagingByWorkspace: Record<string, boolean>;
  threadListCursorByWorkspace: Record<string, string | null>;
  lastAgentMessageByThread: Record<string, { text: string; timestamp: number }>;
  activeWorkspaceId: string | null;
  activeThreadId: string | null;
  accountRateLimits: RateLimitSnapshot | null;
  usageShowRemaining: boolean;
  accountInfo: AccountSnapshot | null;
  onSwitchAccount: () => void;
  onCancelSwitchAccount: () => void;
  accountSwitching: boolean;
  onOpenSettings: () => void;
  onOpenDebug: () => void;
  showDebugButton: boolean;
  onAddWorkspace: () => void;
  onSelectHome: () => void;
  onSelectWorkspace: (id: string) => void;
  onConnectWorkspace: (workspace: WorkspaceInfo) => void;
  onAddAgent: (workspace: WorkspaceInfo) => void;
  onAddWorktreeAgent: (workspace: WorkspaceInfo) => void;
  onAddCloneAgent: (workspace: WorkspaceInfo) => void;
  onToggleWorkspaceCollapse: (workspaceId: string, collapsed: boolean) => void;
  onSelectThread: (workspaceId: string, threadId: string) => void;
  onDeleteThread: (workspaceId: string, threadId: string) => void;
  onSyncThread: (workspaceId: string, threadId: string) => void;
  pinThread: (workspaceId: string, threadId: string) => boolean;
  unpinThread: (workspaceId: string, threadId: string) => void;
  isThreadPinned: (workspaceId: string, threadId: string) => boolean;
  getPinTimestamp: (workspaceId: string, threadId: string) => number | null;
  onRenameThread: (workspaceId: string, threadId: string) => void;
  onDeleteWorkspace: (workspaceId: string) => void;
  onDeleteWorktree: (workspaceId: string) => void;
  onLoadOlderThreads: (workspaceId: string) => void;
  onReloadWorkspaceThreads: (workspaceId: string) => void;
  workspaceDropTargetRef: RefObject<HTMLElement | null>;
  isWorkspaceDropActive: boolean;
  workspaceDropText: string;
  onWorkspaceDragOver: (event: React.DragEvent<HTMLElement>) => void;
  onWorkspaceDragEnter: (event: React.DragEvent<HTMLElement>) => void;
  onWorkspaceDragLeave: (event: React.DragEvent<HTMLElement>) => void;
  onWorkspaceDrop: (event: React.DragEvent<HTMLElement>) => void;
};

export function Sidebar({
  workspaces,
  groupedWorkspaces,
  hasWorkspaceGroups,
  deletingWorktreeIds,
  threadsByWorkspace,
  threadParentById,
  threadStatusById,
  threadListLoadingByWorkspace,
  threadListPagingByWorkspace,
  threadListCursorByWorkspace,
  lastAgentMessageByThread,
  activeWorkspaceId,
  activeThreadId,
  accountRateLimits,
  usageShowRemaining,
  accountInfo,
  onSwitchAccount,
  onCancelSwitchAccount,
  accountSwitching,
  onOpenSettings,
  onOpenDebug,
  showDebugButton,
  onAddWorkspace,
  onSelectHome,
  onSelectWorkspace,
  onConnectWorkspace,
  onAddAgent,
  onAddWorktreeAgent,
  onAddCloneAgent,
  onToggleWorkspaceCollapse,
  onSelectThread,
  onDeleteThread,
  onSyncThread,
  pinThread,
  unpinThread,
  isThreadPinned,
  getPinTimestamp,
  onRenameThread,
  onDeleteWorkspace,
  onDeleteWorktree,
  onLoadOlderThreads,
  onReloadWorkspaceThreads,
  workspaceDropTargetRef,
  isWorkspaceDropActive,
  workspaceDropText,
  onWorkspaceDragOver,
  onWorkspaceDragEnter,
  onWorkspaceDragLeave,
  onWorkspaceDrop,
}: SidebarProps) {
  const [expandedWorkspaces, setExpandedWorkspaces] = useState(
    new Set<string>(),
  );
  const [searchQuery, setSearchQuery] = useState("");
  const [addMenuAnchor, setAddMenuAnchor] = useState<{
    workspaceId: string;
    top: number;
    left: number;
    width: number;
  } | null>(null);
  const addMenuRef = useRef<HTMLDivElement | null>(null);

  const [searchResults, setSearchResults] = useState<
    Array<{ workspaceId: string; thread: ThreadSummary }> | null
  >(null);
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
            const result = (response.result ?? response) as Record<string, unknown>;
            const data = Array.isArray(result?.data)
              ? (result.data as Record<string, unknown>[])
              : [];
            return data.map((thread) => {
              const id = String(thread?.id ?? "");
              const preview = String(thread?.preview ?? "").trim();
              const updatedAt = Number(thread?.updatedAt ?? thread?.createdAt ?? 0);
              return {
                workspaceId: workspace.id,
                thread: {
                  id,
                  name: preview.length > 38 ? `${preview.slice(0, 38)}…` : preview || id.slice(0, 12),
                  updatedAt,
                },
              };
            }).filter((item) => item.thread.id);
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
    }, 300);
    return () => {
      cancelled = true;
      if (searchTimerRef.current) {
        clearTimeout(searchTimerRef.current);
      }
    };
  }, [searchQuery, workspaces]);
  const { collapsedGroups, toggleGroupCollapse } = useCollapsedGroups(
    COLLAPSED_GROUPS_STORAGE_KEY,
  );
  const { getThreadRows } = useThreadRows(threadParentById);
  const { showThreadMenu, showWorkspaceMenu, showWorktreeMenu } =
    useSidebarMenus({
      onDeleteThread,
      onSyncThread,
      onPinThread: pinThread,
      onUnpinThread: unpinThread,
      isThreadPinned,
      onRenameThread,
      onReloadWorkspaceThreads,
      onDeleteWorkspace,
      onDeleteWorktree,
    });
  const {
    sessionPercent,
    weeklyPercent,
    sonnetPercent,
    sessionResetLabel,
    weeklyResetLabel,
    sonnetResetLabel,
    creditsLabel,
    showWeekly,
    showSonnet,
  } = getUsageLabels(accountRateLimits, usageShowRemaining);

  const accountEmail = accountInfo?.email?.trim() ?? "";
  const accountButtonLabel = accountEmail
    ? accountEmail
    : accountInfo?.type === "apikey"
      ? "API key"
      : "Sign in to Claude";
  const accountActionLabel = accountEmail ? "Switch account" : "Sign in";
  const showAccountSwitcher = Boolean(activeWorkspaceId);
  const accountSwitchDisabled = accountSwitching || !activeWorkspaceId;
  const accountCancelDisabled = !accountSwitching || !activeWorkspaceId;

  const pinnedThreadRows = (() => {
    type ThreadRow = { thread: ThreadSummary; depth: number };
    const groups: Array<{
      pinTime: number;
      workspaceId: string;
      rows: ThreadRow[];
    }> = [];

    workspaces.forEach((workspace) => {
      const threads = threadsByWorkspace[workspace.id] ?? [];
      if (!threads.length) {
        return;
      }
      const { pinnedRows } = getThreadRows(
        threads,
        true,
        workspace.id,
        getPinTimestamp,
      );
      if (!pinnedRows.length) {
        return;
      }
      let currentRows: ThreadRow[] = [];
      let currentPinTime: number | null = null;

      pinnedRows.forEach((row) => {
        if (row.depth === 0) {
          if (currentRows.length && currentPinTime !== null) {
            groups.push({
              pinTime: currentPinTime,
              workspaceId: workspace.id,
              rows: currentRows,
            });
          }
          currentRows = [row];
          currentPinTime = getPinTimestamp(workspace.id, row.thread.id);
        } else {
          currentRows.push(row);
        }
      });

      if (currentRows.length && currentPinTime !== null) {
        groups.push({
          pinTime: currentPinTime,
          workspaceId: workspace.id,
          rows: currentRows,
        });
      }
    });

    return groups
      .sort((a, b) => a.pinTime - b.pinTime)
      .flatMap((group) =>
        group.rows.map((row) => ({
          ...row,
          workspaceId: group.workspaceId,
        })),
      );
  })();

  const scrollFadeDeps = useMemo(
    () => [groupedWorkspaces, threadsByWorkspace, expandedWorkspaces],
    [groupedWorkspaces, threadsByWorkspace, expandedWorkspaces],
  );
  const { sidebarBodyRef, scrollFade, updateScrollFade } =
    useSidebarScrollFade(scrollFadeDeps);

  const worktreesByParent = useMemo(() => {
    const worktrees = new Map<string, WorkspaceInfo[]>();
    workspaces
      .filter((entry) => (entry.kind ?? "main") === "worktree" && entry.parentId)
      .forEach((entry) => {
        const parentId = entry.parentId as string;
        const list = worktrees.get(parentId) ?? [];
        list.push(entry);
        worktrees.set(parentId, list);
      });
    worktrees.forEach((entries) => {
      entries.sort((a, b) => a.name.localeCompare(b.name));
    });
    return worktrees;
  }, [workspaces]);

  const handleToggleExpanded = useCallback((workspaceId: string) => {
    setExpandedWorkspaces((prev) => {
      const next = new Set(prev);
      if (next.has(workspaceId)) {
        next.delete(workspaceId);
      } else {
        next.add(workspaceId);
      }
      return next;
    });
  }, []);

  const getThreadTime = useCallback(
    (thread: ThreadSummary) => {
      const lastMessage = lastAgentMessageByThread[thread.id];
      const timestamp = lastMessage?.timestamp ?? thread.updatedAt ?? null;
      return timestamp ? formatRelativeTimeShort(timestamp) : null;
    },
    [lastAgentMessageByThread],
  );

  useEffect(() => {
    if (!addMenuAnchor) {
      return;
    }
    function handlePointerDown(event: Event) {
      const target = event.target as Node | null;
      if (addMenuRef.current && target && addMenuRef.current.contains(target)) {
        return;
      }
      setAddMenuAnchor(null);
    }
    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("scroll", handlePointerDown, true);
    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("scroll", handlePointerDown, true);
    };
  }, [addMenuAnchor]);

  return (
    <aside
      className="sidebar"
      ref={workspaceDropTargetRef}
      onDragOver={onWorkspaceDragOver}
      onDragEnter={onWorkspaceDragEnter}
      onDragLeave={onWorkspaceDragLeave}
      onDrop={onWorkspaceDrop}
    >
      <SidebarHeader
        onSelectHome={onSelectHome}
        onAddWorkspace={onAddWorkspace}
      />
      <div className="sidebar-search" data-tauri-drag-region="false">
        <Search className="sidebar-search-icon" aria-hidden />
        <input
          ref={searchInputRef}
          type="text"
          className="sidebar-search-input"
          placeholder="Search by session ID..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setSearchQuery("");
              searchInputRef.current?.blur();
            }
          }}
        />
        {searchQuery && (
          <button
            className="sidebar-search-clear"
            onClick={() => {
              setSearchQuery("");
              searchInputRef.current?.focus();
            }}
            aria-label="Clear search"
          >
            <X size={12} aria-hidden />
          </button>
        )}
      </div>
      <div
        className={`workspace-drop-overlay${
          isWorkspaceDropActive ? " is-active" : ""
        }`}
        aria-hidden
      >
        <div
          className={`workspace-drop-overlay-text${
            workspaceDropText === "Adding Project..." ? " is-busy" : ""
          }`}
        >
          {workspaceDropText === "Drop Project Here" && (
            <FolderOpen className="workspace-drop-overlay-icon" aria-hidden />
          )}
          {workspaceDropText}
        </div>
      </div>
      <div
        className={`sidebar-body${scrollFade.top ? " fade-top" : ""}${
          scrollFade.bottom ? " fade-bottom" : ""
        }`}
        onScroll={updateScrollFade}
        ref={sidebarBodyRef}
      >
        {searchResults !== null ? (
          <div className="sidebar-search-results">
            {isSearching && (
              <div className="sidebar-search-status">Searching...</div>
            )}
            {!isSearching && searchResults.length === 0 && (
              <div className="sidebar-search-status">No sessions found</div>
            )}
            {searchResults.map(({ workspaceId, thread }) => {
              const workspace = workspaces.find((w) => w.id === workspaceId);
              return (
                <div
                  key={`${workspaceId}:${thread.id}`}
                  className={`thread-row ${
                    workspaceId === activeWorkspaceId && thread.id === activeThreadId
                      ? "active"
                      : ""
                  }`}
                  onClick={() => onSelectThread(workspaceId, thread.id)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onSelectThread(workspaceId, thread.id);
                    }
                  }}
                >
                  <span className="thread-status ready" aria-hidden />
                  <span className="thread-name">
                    <span className="sidebar-search-result-id">
                      {thread.id.slice(0, 8)}
                    </span>
                    {" "}
                    {thread.name}
                  </span>
                  {workspace && (
                    <div className="thread-meta">
                      <span className="thread-time">
                        {workspace.name}
                      </span>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        ) : (
        <div className="workspace-list">
          {pinnedThreadRows.length > 0 && (
            <div className="pinned-section">
              <div className="workspace-group-header">
                <div className="workspace-group-label">Pinned</div>
              </div>
              <PinnedThreadList
                rows={pinnedThreadRows}
                activeWorkspaceId={activeWorkspaceId}
                activeThreadId={activeThreadId}
                threadStatusById={threadStatusById}
                getThreadTime={getThreadTime}
                isThreadPinned={isThreadPinned}
                onSelectThread={onSelectThread}
                onShowThreadMenu={showThreadMenu}
              />
            </div>
          )}
          {groupedWorkspaces.map((group) => {
            const groupId = group.id;
            const showGroupHeader = Boolean(groupId) || hasWorkspaceGroups;
            const toggleId = groupId ?? (showGroupHeader ? UNGROUPED_COLLAPSE_ID : null);
            const isGroupCollapsed = Boolean(
              toggleId && collapsedGroups.has(toggleId),
            );

            return (
              <WorkspaceGroup
                key={group.id ?? "ungrouped"}
                toggleId={toggleId}
                name={group.name}
                showHeader={showGroupHeader}
                isCollapsed={isGroupCollapsed}
                onToggleCollapse={toggleGroupCollapse}
              >
                {group.workspaces.map((entry) => {
                  const threads = threadsByWorkspace[entry.id] ?? [];
                  const isCollapsed = entry.settings.sidebarCollapsed;
                  const isExpanded = expandedWorkspaces.has(entry.id);
                  const {
                    unpinnedRows,
                    totalRoots: totalThreadRoots,
                  } = getThreadRows(
                    threads,
                    isExpanded,
                    entry.id,
                    getPinTimestamp,
                  );
                  const nextCursor =
                    threadListCursorByWorkspace[entry.id] ?? null;
                  const showThreadList =
                    !isCollapsed && (threads.length > 0 || Boolean(nextCursor));
                  const isLoadingThreads =
                    threadListLoadingByWorkspace[entry.id] ?? false;
                  const showThreadLoader =
                    !isCollapsed && isLoadingThreads && threads.length === 0;
                  const isPaging = threadListPagingByWorkspace[entry.id] ?? false;
                  const worktrees = worktreesByParent.get(entry.id) ?? [];
                  const addMenuOpen = addMenuAnchor?.workspaceId === entry.id;

                  return (
                    <WorkspaceCard
                      key={entry.id}
                      workspace={entry}
                      workspaceName={entry.name}
                      isActive={entry.id === activeWorkspaceId}
                      isCollapsed={isCollapsed}
                      addMenuOpen={addMenuOpen}
                      addMenuWidth={ADD_MENU_WIDTH}
                      onSelectWorkspace={onSelectWorkspace}
                      onShowWorkspaceMenu={showWorkspaceMenu}
                      onToggleWorkspaceCollapse={onToggleWorkspaceCollapse}
                      onConnectWorkspace={onConnectWorkspace}
                      onToggleAddMenu={setAddMenuAnchor}
                    >
                      {addMenuOpen && addMenuAnchor &&
                        createPortal(
                          <div
                            className="workspace-add-menu popover-surface"
                            ref={addMenuRef}
                            style={{
                              top: addMenuAnchor.top,
                              left: addMenuAnchor.left,
                              width: addMenuAnchor.width,
                            }}
                          >
                            <button
                              className="workspace-add-option"
                              onClick={(event) => {
                                event.stopPropagation();
                                setAddMenuAnchor(null);
                                onAddAgent(entry);
                              }}
                            >
                              New agent
                            </button>
                            <button
                              className="workspace-add-option"
                              onClick={(event) => {
                                event.stopPropagation();
                                setAddMenuAnchor(null);
                                onAddWorktreeAgent(entry);
                              }}
                            >
                              New worktree agent
                            </button>
                            <button
                              className="workspace-add-option"
                              onClick={(event) => {
                                event.stopPropagation();
                                setAddMenuAnchor(null);
                                onAddCloneAgent(entry);
                              }}
                            >
                              New clone agent
                            </button>
                          </div>,
                          document.body,
                        )}
                      {!isCollapsed && worktrees.length > 0 && (
                        <WorktreeSection
                          worktrees={worktrees}
                          deletingWorktreeIds={deletingWorktreeIds}
                          threadsByWorkspace={threadsByWorkspace}
                          threadStatusById={threadStatusById}
                          threadListLoadingByWorkspace={threadListLoadingByWorkspace}
                          threadListPagingByWorkspace={threadListPagingByWorkspace}
                          threadListCursorByWorkspace={threadListCursorByWorkspace}
                          expandedWorkspaces={expandedWorkspaces}
                          activeWorkspaceId={activeWorkspaceId}
                          activeThreadId={activeThreadId}
                          getThreadRows={getThreadRows}
                          getThreadTime={getThreadTime}
                          isThreadPinned={isThreadPinned}
                          getPinTimestamp={getPinTimestamp}
                          onSelectWorkspace={onSelectWorkspace}
                          onConnectWorkspace={onConnectWorkspace}
                          onToggleWorkspaceCollapse={onToggleWorkspaceCollapse}
                          onSelectThread={onSelectThread}
                          onShowThreadMenu={showThreadMenu}
                          onShowWorktreeMenu={showWorktreeMenu}
                          onToggleExpanded={handleToggleExpanded}
                          onLoadOlderThreads={onLoadOlderThreads}
                        />
                      )}
                      {showThreadList && (
                        <ThreadList
                          workspaceId={entry.id}
                          pinnedRows={[]}
                          unpinnedRows={unpinnedRows}
                          totalThreadRoots={totalThreadRoots}
                          isExpanded={isExpanded}
                          nextCursor={nextCursor}
                          isPaging={isPaging}
                          activeWorkspaceId={activeWorkspaceId}
                          activeThreadId={activeThreadId}
                          threadStatusById={threadStatusById}
                          getThreadTime={getThreadTime}
                          isThreadPinned={isThreadPinned}
                          onToggleExpanded={handleToggleExpanded}
                          onLoadOlderThreads={onLoadOlderThreads}
                          onSelectThread={onSelectThread}
                          onShowThreadMenu={showThreadMenu}
                        />
                      )}
                      {showThreadLoader && <ThreadLoading />}
                    </WorkspaceCard>
                  );
                })}
              </WorkspaceGroup>
            );
          })}
          {!groupedWorkspaces.length && (
            <div className="empty">Add a workspace to start.</div>
          )}
        </div>
        )}
      </div>
      <SidebarFooter
        sessionPercent={sessionPercent}
        weeklyPercent={weeklyPercent}
        sonnetPercent={sonnetPercent}
        sessionResetLabel={sessionResetLabel}
        weeklyResetLabel={weeklyResetLabel}
        sonnetResetLabel={sonnetResetLabel}
        creditsLabel={creditsLabel}
        showWeekly={showWeekly}
        showSonnet={showSonnet}
      />
      <SidebarCornerActions
        onOpenSettings={onOpenSettings}
        onOpenDebug={onOpenDebug}
        showDebugButton={showDebugButton}
        showAccountSwitcher={showAccountSwitcher}
        accountLabel={accountButtonLabel}
        accountActionLabel={accountActionLabel}
        accountDisabled={accountSwitchDisabled}
        accountSwitching={accountSwitching}
        accountCancelDisabled={accountCancelDisabled}
        onSwitchAccount={onSwitchAccount}
        onCancelSwitchAccount={onCancelSwitchAccount}
      />
    </aside>
  );
}
