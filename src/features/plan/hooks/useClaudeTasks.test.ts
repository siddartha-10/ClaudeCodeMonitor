// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useClaudeTasks } from "./useClaudeTasks";

vi.mock("../../../services/tauri", () => ({
  getClaudeTasks: vi.fn(),
}));

import { getClaudeTasks } from "../../../services/tauri";

const mockGetClaudeTasks = vi.mocked(getClaudeTasks);

describe("useClaudeTasks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
          activeForm: null,
          status: "completed",
          blocks: [],
          blockedBy: [],
        },
        {
          id: "2",
          subject: "Task B",
          description: "",
          activeForm: null,
          status: "in_progress",
          blocks: [],
          blockedBy: [],
        },
        {
          id: "3",
          subject: "Task C",
          description: "",
          activeForm: null,
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
          activeForm: null,
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
});
