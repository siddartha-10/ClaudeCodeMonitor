// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRef } from "react";
import { Sidebar } from "./Sidebar";

afterEach(() => {
  cleanup();
});

const baseProps = {
  workspaces: [],
  groupedWorkspaces: [],
  hasWorkspaceGroups: false,
  deletingWorktreeIds: new Set<string>(),
  threadsByWorkspace: {},
  threadParentById: {},
  threadStatusById: {},
  threadListLoadingByWorkspace: {},
  threadListPagingByWorkspace: {},
  threadListCursorByWorkspace: {},
  lastAgentMessageByThread: {},
  activeWorkspaceId: null,
  activeThreadId: null,
  accountRateLimits: null,
  onOpenSettings: vi.fn(),
  onOpenDebug: vi.fn(),
  showDebugButton: false,
  onAddWorkspace: vi.fn(),
  onSelectHome: vi.fn(),
  onSelectWorkspace: vi.fn(),
  onConnectWorkspace: vi.fn(),
  onAddAgent: vi.fn(),
  onAddWorktreeAgent: vi.fn(),
  onAddCloneAgent: vi.fn(),
  onToggleWorkspaceCollapse: vi.fn(),
  onSelectThread: vi.fn(),
  onDeleteThread: vi.fn(),
  onSyncThread: vi.fn(),
  pinThread: vi.fn(() => false),
  unpinThread: vi.fn(),
  isThreadPinned: vi.fn(() => false),
  getPinTimestamp: vi.fn(() => null),
  onRenameThread: vi.fn(),
  onDeleteWorkspace: vi.fn(),
  onDeleteWorktree: vi.fn(),
  onLoadOlderThreads: vi.fn(),
  onReloadWorkspaceThreads: vi.fn(),
  workspaceDropTargetRef: createRef<HTMLElement>(),
  isWorkspaceDropActive: false,
  workspaceDropText: "Drop Project Here",
  onWorkspaceDragOver: vi.fn(),
  onWorkspaceDragEnter: vi.fn(),
  onWorkspaceDragLeave: vi.fn(),
  onWorkspaceDrop: vi.fn(),
  usageShowRemaining: false,
  accountInfo: null,
  onSwitchAccount: vi.fn(),
  onCancelSwitchAccount: vi.fn(),
  accountSwitching: false,
};

describe("Sidebar", () => {
  it("renders the always-visible session search input", () => {
    render(<Sidebar {...baseProps} />);

    // Search input should always be visible
    const input = screen.getByPlaceholderText("Search by session ID...");
    expect(input).toBeTruthy();
  });

  it("allows typing in the search input and clearing it", () => {
    vi.useFakeTimers();
    render(<Sidebar {...baseProps} />);

    const input = screen.getByPlaceholderText("Search by session ID...") as HTMLInputElement;

    // Type in the search input
    act(() => {
      fireEvent.change(input, { target: { value: "abc123" } });
      vi.runOnlyPendingTimers();
    });
    expect(input.value).toBe("abc123");

    // Clear button should appear when there's text
    const clearButton = screen.getByLabelText("Clear search");
    expect(clearButton).toBeTruthy();

    // Clicking clear should empty the input
    act(() => {
      fireEvent.click(clearButton);
      vi.runOnlyPendingTimers();
    });
    expect(input.value).toBe("");

    vi.useRealTimers();
  });

  it("clears search on Escape key", () => {
    vi.useFakeTimers();
    render(<Sidebar {...baseProps} />);

    const input = screen.getByPlaceholderText("Search by session ID...") as HTMLInputElement;

    act(() => {
      fireEvent.change(input, { target: { value: "test" } });
      vi.runOnlyPendingTimers();
    });
    expect(input.value).toBe("test");

    act(() => {
      fireEvent.keyDown(input, { key: "Escape" });
      vi.runOnlyPendingTimers();
    });
    expect(input.value).toBe("");

    vi.useRealTimers();
  });
});
