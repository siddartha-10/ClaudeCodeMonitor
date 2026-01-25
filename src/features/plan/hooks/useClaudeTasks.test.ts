// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { useClaudeTasks } from "./useClaudeTasks";

// Mock the tauri service
vi.mock("../../../services/tauri", () => ({
  getClaudeTasks: vi.fn(),
}));

// Mock @tauri-apps/api/core
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

// Mock @tauri-apps/api/event
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

import { getClaudeTasks } from "../../../services/tauri";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const mockGetClaudeTasks = vi.mocked(getClaudeTasks);
const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

describe("useClaudeTasks", () => {
  let mockUnlisten: Mock;

  beforeEach(() => {
    vi.clearAllMocks();

    // Default mock implementations
    mockInvoke.mockResolvedValue(undefined);
    mockUnlisten = vi.fn();
    mockListen.mockResolvedValue(mockUnlisten);
  });

  it("returns empty tasks when no thread is active", () => {
    const { result } = renderHook(() =>
      useClaudeTasks({ activeThreadId: null, isProcessing: false })
    );

    expect(result.current.tasks).toEqual([]);
    expect(result.current.plan).toBeNull();
  });

  it("fetches tasks when thread becomes active", async () => {
    mockGetClaudeTasks.mockResolvedValue({
      sessionId: "test-thread",
      tasks: [
        {
          id: "1",
          subject: "First task",
          description: "Description",
          activeForm: "Working",
          status: "pending",
          blocks: [],
          blockedBy: [],
        },
      ],
    });

    const { result } = renderHook(() =>
      useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
    );

    await waitFor(() => {
      expect(result.current.tasks.length).toBe(1);
    });

    expect(result.current.tasks[0].subject).toBe("First task");
    expect(mockGetClaudeTasks).toHaveBeenCalledWith("test-thread");
  });

  it("converts tasks to TurnPlan format", async () => {
    mockGetClaudeTasks.mockResolvedValue({
      sessionId: "test-thread",
      tasks: [
        {
          id: "1",
          subject: "Task A",
          description: "",
          activeForm: undefined,
          status: "completed",
          blocks: [],
          blockedBy: [],
        },
        {
          id: "2",
          subject: "Task B",
          description: "",
          activeForm: undefined,
          status: "in_progress",
          blocks: [],
          blockedBy: [],
        },
        {
          id: "3",
          subject: "Task C",
          description: "",
          activeForm: undefined,
          status: "pending",
          blocks: [],
          blockedBy: [],
        },
      ],
    });

    const { result } = renderHook(() =>
      useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
    );

    await waitFor(() => {
      expect(result.current.plan).not.toBeNull();
    });

    expect(result.current.plan?.steps).toEqual([
      { step: "Task A", status: "completed" },
      { step: "Task B", status: "inProgress" },
      { step: "Task C", status: "pending" },
    ]);
  });

  it("clears tasks when thread changes", async () => {
    mockGetClaudeTasks.mockResolvedValue({
      sessionId: "thread-1",
      tasks: [
        {
          id: "1",
          subject: "Old task",
          description: "",
          activeForm: undefined,
          status: "pending",
          blocks: [],
          blockedBy: [],
        },
      ],
    });

    const { result, rerender } = renderHook(
      ({ threadId }) =>
        useClaudeTasks({ activeThreadId: threadId, isProcessing: false }),
      { initialProps: { threadId: "thread-1" as string | null } }
    );

    await waitFor(() => {
      expect(result.current.tasks.length).toBe(1);
    });

    // Change to null thread
    rerender({ threadId: null });

    expect(result.current.tasks).toEqual([]);
    expect(result.current.plan).toBeNull();
  });

  it("handles fetch errors gracefully", async () => {
    mockGetClaudeTasks.mockRejectedValue(new Error("Network error"));

    const { result } = renderHook(() =>
      useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
    );

    // Should not throw, tasks remain empty
    await waitFor(() => {
      expect(mockGetClaudeTasks).toHaveBeenCalled();
    });

    expect(result.current.tasks).toEqual([]);
  });

  describe("file watcher integration", () => {
    it("starts watcher when thread becomes active", async () => {
      mockGetClaudeTasks.mockResolvedValue({ sessionId: "test", tasks: [] });

      renderHook(() =>
        useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
      );

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith("task_watcher_start", {
          listId: "test-thread",
        });
      });
    });

    it("sets up listener for task changes", async () => {
      mockGetClaudeTasks.mockResolvedValue({ sessionId: "test", tasks: [] });

      renderHook(() =>
        useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
      );

      await waitFor(() => {
        expect(mockListen).toHaveBeenCalledWith(
          "task-list-changed:test-thread",
          expect.any(Function)
        );
      });
    });

    it("stops watcher and removes listener on unmount", async () => {
      mockGetClaudeTasks.mockResolvedValue({ sessionId: "test", tasks: [] });

      const { unmount } = renderHook(() =>
        useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
      );

      await waitFor(() => {
        expect(mockListen).toHaveBeenCalled();
      });

      unmount();

      await waitFor(() => {
        expect(mockUnlisten).toHaveBeenCalled();
        expect(mockInvoke).toHaveBeenCalledWith("task_watcher_stop", {
          listId: "test-thread",
        });
      });
    });

    it("stops old watcher and starts new one when thread changes", async () => {
      mockGetClaudeTasks.mockResolvedValue({ sessionId: "test", tasks: [] });

      const { rerender } = renderHook(
        ({ threadId }) =>
          useClaudeTasks({ activeThreadId: threadId, isProcessing: false }),
        { initialProps: { threadId: "thread-1" as string | null } }
      );

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith("task_watcher_start", {
          listId: "thread-1",
        });
      });

      // Clear to track new calls
      mockInvoke.mockClear();
      mockListen.mockClear();

      // Change to different thread
      rerender({ threadId: "thread-2" });

      await waitFor(() => {
        // Should stop old watcher
        expect(mockInvoke).toHaveBeenCalledWith("task_watcher_stop", {
          listId: "thread-1",
        });
        // Should start new watcher
        expect(mockInvoke).toHaveBeenCalledWith("task_watcher_start", {
          listId: "thread-2",
        });
      });
    });

    it("refetches tasks when file change event is received", async () => {
      mockGetClaudeTasks.mockResolvedValue({ sessionId: "test", tasks: [] });

      let capturedCallback: (() => void) | null = null;
      mockListen.mockImplementation(async (_event, callback) => {
        capturedCallback = callback as () => void;
        return mockUnlisten;
      });

      renderHook(() =>
        useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
      );

      await waitFor(() => {
        expect(mockGetClaudeTasks).toHaveBeenCalledTimes(1);
      });

      // Simulate file change event
      expect(capturedCallback).not.toBeNull();

      mockGetClaudeTasks.mockResolvedValue({
        sessionId: "test-thread",
        tasks: [
          {
            id: "1",
            subject: "New task",
            description: "",
            status: "pending",
            blocks: [],
            blockedBy: [],
          },
        ],
      });

      act(() => {
        capturedCallback!();
      });

      await waitFor(() => {
        expect(mockGetClaudeTasks).toHaveBeenCalledTimes(2);
      });
    });

    it("handles watcher start failure gracefully", async () => {
      mockGetClaudeTasks.mockResolvedValue({ sessionId: "test", tasks: [] });
      mockInvoke.mockRejectedValue(new Error("Watcher failed"));

      // Should not throw
      const { result } = renderHook(() =>
        useClaudeTasks({ activeThreadId: "test-thread", isProcessing: false })
      );

      await waitFor(() => {
        expect(mockInvoke).toHaveBeenCalledWith("task_watcher_start", {
          listId: "test-thread",
        });
      });

      // Hook should still work, just without real-time updates
      expect(result.current.tasks).toEqual([]);
    });
  });
});
