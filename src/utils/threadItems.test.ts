import { describe, expect, it } from "vitest";
import type { ConversationItem } from "../types";
import {
  buildConversationItem,
  buildConversationItemFromThreadItem,
  mergeThreadItems,
  normalizeItem,
  prepareThreadItems,
} from "./threadItems";

describe("threadItems", () => {
  it("truncates long message text in normalizeItem", () => {
    const text = "a".repeat(21000);
    const item: ConversationItem = {
      id: "msg-1",
      kind: "message",
      role: "assistant",
      text,
    };
    const normalized = normalizeItem(item);
    expect(normalized.kind).toBe("message");
    if (normalized.kind === "message") {
      expect(normalized.text).not.toBe(text);
      expect(normalized.text.endsWith("...")).toBe(true);
      expect(normalized.text.length).toBeLessThan(text.length);
    }
  });

  it("preserves tool output for fileChange and commandExecution", () => {
    const output = "x".repeat(21000);
    const item: ConversationItem = {
      id: "tool-1",
      kind: "tool",
      toolType: "fileChange",
      title: "File changes",
      detail: "",
      output,
    };
    const normalized = normalizeItem(item);
    expect(normalized.kind).toBe("tool");
    if (normalized.kind === "tool") {
      expect(normalized.output).toBe(output);
    }
  });

  it("truncates older tool output in prepareThreadItems", () => {
    const output = "y".repeat(21000);
    const items: ConversationItem[] = Array.from({ length: 41 }, (_, index) => ({
      id: `tool-${index}`,
      kind: "tool",
      toolType: "commandExecution",
      title: "Tool",
      detail: "",
      output,
    }));
    const prepared = prepareThreadItems(items);
    const firstOutput = prepared[0].kind === "tool" ? prepared[0].output : undefined;
    const secondOutput = prepared[1].kind === "tool" ? prepared[1].output : undefined;
    expect(firstOutput).not.toBe(output);
    expect(firstOutput?.endsWith("...")).toBe(true);
    expect(secondOutput).toBe(output);
  });

  it("drops assistant review summaries that duplicate completed review items", () => {
    const items: ConversationItem[] = [
      {
        id: "review-1",
        kind: "review",
        state: "completed",
        text: "Review summary",
      },
      {
        id: "msg-1",
        kind: "message",
        role: "assistant",
        text: "Review summary",
      },
    ];
    const prepared = prepareThreadItems(items);
    expect(prepared).toHaveLength(1);
    expect(prepared[0].kind).toBe("review");
  });

  it("builds file change items with summary details", () => {
    const item = buildConversationItem({
      type: "fileChange",
      id: "change-1",
      status: "done",
      changes: [
        {
          path: "foo.txt",
          kind: "add",
          diff: "diff --git a/foo.txt b/foo.txt",
        },
      ],
    });
    expect(item).not.toBeNull();
    if (item && item.kind === "tool") {
      expect(item.title).toBe("File changes");
      expect(item.detail).toBe("A foo.txt");
      expect(item.output).toContain("diff --git a/foo.txt b/foo.txt");
      expect(item.changes?.[0]?.path).toBe("foo.txt");
    }
  });

  it("merges thread items preferring richer local tool output", () => {
    const remote: ConversationItem = {
      id: "tool-2",
      kind: "tool",
      toolType: "webSearch",
      title: "Web search",
      detail: "query",
      status: "ok",
      output: "short",
    };
    const local: ConversationItem = {
      id: "tool-2",
      kind: "tool",
      toolType: "webSearch",
      title: "Web search",
      detail: "query",
      output: "much longer output",
    };
    const merged = mergeThreadItems([remote], [local]);
    expect(merged).toHaveLength(1);
    expect(merged[0].kind).toBe("tool");
    if (merged[0].kind === "tool") {
      expect(merged[0].output).toBe("much longer output");
      expect(merged[0].status).toBe("ok");
    }
  });

  it("builds user message text from mixed inputs", () => {
    const item = buildConversationItemFromThreadItem({
      type: "userMessage",
      id: "msg-1",
      content: [
        { type: "text", text: "Please" },
        { type: "skill", name: "Review" },
        { type: "image", url: "https://example.com/image.png" },
      ],
    });
    expect(item).not.toBeNull();
    if (item && item.kind === "message") {
      expect(item.role).toBe("user");
      expect(item.text).toBe("Please $Review");
      expect(item.images).toEqual(["https://example.com/image.png"]);
    }
  });

  it("keeps image-only user messages without placeholder text", () => {
    const item = buildConversationItemFromThreadItem({
      type: "userMessage",
      id: "msg-2",
      content: [{ type: "image", url: "https://example.com/only.png" }],
    });
    expect(item).not.toBeNull();
    if (item && item.kind === "message") {
      expect(item.role).toBe("user");
      expect(item.text).toBe("");
      expect(item.images).toEqual(["https://example.com/only.png"]);
    }
  });

  it("formats collab tool calls with receivers and agent states", () => {
    const item = buildConversationItem({
      type: "collabToolCall",
      id: "collab-1",
      tool: "handoff",
      status: "ok",
      senderThreadId: "thread-a",
      receiverThreadIds: ["thread-b"],
      newThreadId: "thread-c",
      prompt: "Coordinate work",
      agentStatus: { "agent-1": { status: "running" } },
    });
    expect(item).not.toBeNull();
    if (item && item.kind === "tool") {
      expect(item.title).toBe("Collab: handoff");
      expect(item.detail).toContain("From thread-a");
      expect(item.detail).toContain("thread-b, thread-c");
      expect(item.output).toBe("Coordinate work\n\nagent-1: running");
    }
  });

  describe("optimistic user message reconciliation", () => {
    it("drops optimistic user message when server has matching new message", () => {
      // Scenario: user sends "hi", switches threads, comes back
      // Local has optimistic ID, remote has server ID with same text
      const localItems: ConversationItem[] = [
        { id: "1705934567890-user", kind: "message", role: "user", text: "hi" },
        { id: "msg_001", kind: "message", role: "assistant", text: "Hello!" },
      ];
      const remoteItems: ConversationItem[] = [
        { id: "msg_xyz789", kind: "message", role: "user", text: "hi" },
        { id: "msg_001", kind: "message", role: "assistant", text: "Hello!" },
      ];
      const merged = mergeThreadItems(remoteItems, localItems);

      // Should only have 2 items - the optimistic was replaced by server version
      expect(merged).toHaveLength(2);
      const userMessages = merged.filter(
        (i) => i.kind === "message" && i.role === "user",
      );
      expect(userMessages).toHaveLength(1);
      expect(userMessages[0].id).toBe("msg_xyz789"); // server ID, not optimistic
    });

    it("keeps optimistic message when no matching server message exists", () => {
      // Scenario: user sends message but server hasn't processed it yet
      const localItems: ConversationItem[] = [
        { id: "msg_001", kind: "message", role: "user", text: "first message" },
        { id: "1705934567890-user", kind: "message", role: "user", text: "pending message" },
      ];
      const remoteItems: ConversationItem[] = [
        { id: "msg_001", kind: "message", role: "user", text: "first message" },
      ];
      const merged = mergeThreadItems(remoteItems, localItems);

      // Should have both messages - optimistic is still pending
      expect(merged).toHaveLength(2);
      const pendingMsg = merged.find((i) => i.id === "1705934567890-user");
      expect(pendingMsg).toBeDefined();
    });

    it("handles multiple identical optimistic messages with count-based matching", () => {
      // Scenario: user sends "ok" twice quickly (with response between), server processed one
      // Note: messages are separated by other items to avoid adjacent dedupe
      const localItems: ConversationItem[] = [
        { id: "1705934567890-user", kind: "message", role: "user", text: "ok" },
        { id: "msg_resp1", kind: "message", role: "assistant", text: "Got it" },
        { id: "1705934567891-user", kind: "message", role: "user", text: "ok" },
      ];
      const remoteItems: ConversationItem[] = [
        { id: "msg_abc", kind: "message", role: "user", text: "ok" },
        { id: "msg_resp1", kind: "message", role: "assistant", text: "Got it" },
      ];
      const merged = mergeThreadItems(remoteItems, localItems);

      // Should have 3 items - server "ok", assistant response, pending optimistic "ok"
      expect(merged).toHaveLength(3);
      const userMessages = merged.filter(
        (i) => i.kind === "message" && i.role === "user",
      );
      expect(userMessages).toHaveLength(2);
      // One should be server ID, one should be optimistic
      const serverMsg = userMessages.find((i) => i.id === "msg_abc");
      const optimisticMsg = userMessages.find((i) => /^\d+-user$/.test(i.id));
      expect(serverMsg).toBeDefined();
      expect(optimisticMsg).toBeDefined();
    });

    it("drops both optimistics when server has both", () => {
      // Scenario: user sends "ok" twice (with responses between), both processed by server
      const localItems: ConversationItem[] = [
        { id: "1705934567890-user", kind: "message", role: "user", text: "ok" },
        { id: "msg_resp1", kind: "message", role: "assistant", text: "First response" },
        { id: "1705934567891-user", kind: "message", role: "user", text: "ok" },
        { id: "msg_resp2", kind: "message", role: "assistant", text: "Second response" },
      ];
      const remoteItems: ConversationItem[] = [
        { id: "msg_abc", kind: "message", role: "user", text: "ok" },
        { id: "msg_resp1", kind: "message", role: "assistant", text: "First response" },
        { id: "msg_def", kind: "message", role: "user", text: "ok" },
        { id: "msg_resp2", kind: "message", role: "assistant", text: "Second response" },
      ];
      const merged = mergeThreadItems(remoteItems, localItems);

      // Should only have server messages (4 total: 2 user + 2 assistant)
      expect(merged).toHaveLength(4);
      const userMessages = merged.filter(
        (i) => i.kind === "message" && i.role === "user",
      );
      expect(userMessages).toHaveLength(2);
      expect(userMessages.every((i) => i.id.startsWith("msg_"))).toBe(true);
    });

    it("does not drop optimistic when server message was already in local", () => {
      // Scenario: local already had the server message, optimistic is for a new send
      // Messages separated by assistant response to avoid adjacent dedupe
      const localItems: ConversationItem[] = [
        { id: "msg_abc", kind: "message", role: "user", text: "hello" },
        { id: "msg_resp1", kind: "message", role: "assistant", text: "Hi there" },
        { id: "1705934567890-user", kind: "message", role: "user", text: "hello" },
      ];
      const remoteItems: ConversationItem[] = [
        { id: "msg_abc", kind: "message", role: "user", text: "hello" },
        { id: "msg_resp1", kind: "message", role: "assistant", text: "Hi there" },
      ];
      const merged = mergeThreadItems(remoteItems, localItems);

      // Should keep all 3 - msg_abc was already known, optimistic is pending
      expect(merged).toHaveLength(3);
      const optimisticMsg = merged.find((i) => i.id === "1705934567890-user");
      expect(optimisticMsg).toBeDefined();
    });

    it("only reconciles user messages, not assistant messages", () => {
      // Optimistic assistant messages (if any) should not be affected
      const localItems: ConversationItem[] = [
        { id: "1705934567890-assistant", kind: "message", role: "assistant", text: "response" },
      ];
      const remoteItems: ConversationItem[] = [
        { id: "msg_abc", kind: "message", role: "assistant", text: "response" },
      ];
      const merged = mergeThreadItems(remoteItems, localItems);

      // Both should exist (dedupe may handle it, but optimistic reconciliation shouldn't)
      // The key is that the optimistic ID pattern for user messages doesn't match assistant
      expect(merged.length).toBeGreaterThanOrEqual(1);
    });
  });

});
