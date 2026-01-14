import type { MouseEvent, ReactNode, RefObject } from "react";
import { ArrowLeft } from "lucide-react";
import { Sidebar } from "../components/Sidebar";
import { Home } from "../components/Home";
import { MainHeader } from "../components/MainHeader";
import { Messages } from "../components/Messages";
import { ApprovalToasts } from "../components/ApprovalToasts";
import { UpdateToast } from "../components/UpdateToast";
import { Composer } from "../components/Composer";
import { GitDiffPanel } from "../components/GitDiffPanel";
import { GitDiffViewer } from "../components/GitDiffViewer";
import { DebugPanel } from "../components/DebugPanel";
import { PlanPanel } from "../components/PlanPanel";
import { TabBar } from "../components/TabBar";
import { TabletNav } from "../components/TabletNav";
import type {
  AccessMode,
  ApprovalRequest,
  BranchInfo,
  ConversationItem,
  CustomPromptOption,
  DebugEntry,
  DiffLineReference,
  GitFileStatus,
  GitHubIssue,
  GitLogEntry,
  ModelOption,
  QueuedMessage,
  RateLimitSnapshot,
  SkillOption,
  ThreadSummary,
  ThreadTokenUsage,
  TurnPlan,
  WorkspaceInfo,
} from "../types";
import type { UpdateState } from "./useUpdater";

type ThreadActivityStatus = {
  isProcessing: boolean;
  hasUnread: boolean;
  isReviewing: boolean;
  processingStartedAt?: number | null;
  lastDurationMs?: number | null;
};

type GitDiffViewerItem = {
  path: string;
  status: string;
  diff: string;
};

type LayoutNodesOptions = {
  workspaces: WorkspaceInfo[];
  threadsByWorkspace: Record<string, ThreadSummary[]>;
  threadStatusById: Record<string, ThreadActivityStatus>;
  threadListLoadingByWorkspace: Record<string, boolean>;
  activeWorkspaceId: string | null;
  activeThreadId: string | null;
  activeItems: ConversationItem[];
  activeRateLimits: RateLimitSnapshot | null;
  approvals: ApprovalRequest[];
  handleApprovalDecision: (
    request: ApprovalRequest,
    decision: "accept" | "decline",
  ) => void;
  onOpenSettings: () => void;
  onOpenDebug: () => void;
  hasDebugAlerts: boolean;
  onAddWorkspace: () => void;
  onSelectHome: () => void;
  onSelectWorkspace: (workspaceId: string) => void;
  onConnectWorkspace: (workspace: WorkspaceInfo) => Promise<void>;
  onAddAgent: (workspace: WorkspaceInfo) => Promise<void>;
  onAddWorktreeAgent: (workspace: WorkspaceInfo) => Promise<void>;
  onToggleWorkspaceCollapse: (workspaceId: string, collapsed: boolean) => void;
  onSelectThread: (workspaceId: string, threadId: string) => void;
  onDeleteThread: (workspaceId: string, threadId: string) => void;
  onDeleteWorkspace: (workspaceId: string) => void;
  onDeleteWorktree: (workspaceId: string) => void;
  updaterState: UpdateState;
  onUpdate: () => void;
  onDismissUpdate: () => void;
  latestAgentRuns: Array<{
    threadId: string;
    message: string;
    timestamp: number;
    projectName: string;
    workspaceId: string;
    isProcessing: boolean;
  }>;
  isLoadingLatestAgents: boolean;
  onSelectHomeThread: (workspaceId: string, threadId: string) => void;
  activeWorkspace: WorkspaceInfo | null;
  activeParentWorkspace: WorkspaceInfo | null;
  worktreeLabel: string | null;
  isWorktreeWorkspace: boolean;
  branchName: string;
  branches: BranchInfo[];
  onCheckoutBranch: (name: string) => Promise<void>;
  onCreateBranch: (name: string) => Promise<void>;
  centerMode: "chat" | "diff";
  onExitDiff: () => void;
  activeTab: "projects" | "codex" | "git" | "log";
  onSelectTab: (tab: "projects" | "codex" | "git" | "log") => void;
  tabletNavTab: "codex" | "git" | "log";
  gitPanelMode: "diff" | "log" | "issues";
  onGitPanelModeChange: (mode: "diff" | "log" | "issues") => void;
  gitStatus: {
    branchName: string;
    files: GitFileStatus[];
    totalAdditions: number;
    totalDeletions: number;
    error: string | null;
  };
  fileStatus: string;
  selectedDiffPath: string | null;
  onSelectDiff: (path: string) => void;
  gitLogEntries: GitLogEntry[];
  gitLogTotal: number;
  gitLogAhead: number;
  gitLogBehind: number;
  gitLogAheadEntries: GitLogEntry[];
  gitLogBehindEntries: GitLogEntry[];
  gitLogUpstream: string | null;
  gitLogError: string | null;
  gitLogLoading: boolean;
  gitIssues: GitHubIssue[];
  gitIssuesTotal: number;
  gitIssuesLoading: boolean;
  gitIssuesError: string | null;
  gitRemoteUrl: string | null;
  gitDiffs: GitDiffViewerItem[];
  gitDiffLoading: boolean;
  gitDiffError: string | null;
  onDiffLineReference: (reference: DiffLineReference) => void;
  onDiffActivePathChange: (path: string) => void;
  onSend: (text: string, images: string[]) => void | Promise<void>;
  onStop: () => void;
  canStop: boolean;
  isReviewing: boolean;
  isProcessing: boolean;
  activeTokenUsage: ThreadTokenUsage | null;
  activeQueue: QueuedMessage[];
  draftText: string;
  onDraftChange: (next: string) => void;
  activeImages: string[];
  onPickImages: () => void | Promise<void>;
  onAttachImages: (paths: string[]) => void;
  onRemoveImage: (path: string) => void;
  prefillDraft: QueuedMessage | null;
  onPrefillHandled: (id: string) => void;
  insertText: QueuedMessage | null;
  onInsertHandled: (id: string) => void;
  onEditQueued: (item: QueuedMessage) => void;
  onDeleteQueued: (id: string) => void;
  models: ModelOption[];
  selectedModelId: string | null;
  onSelectModel: (id: string | null) => void;
  reasoningOptions: string[];
  selectedEffort: string | null;
  onSelectEffort: (effort: string | null) => void;
  accessMode: AccessMode;
  onSelectAccessMode: (mode: AccessMode) => void;
  skills: SkillOption[];
  prompts: CustomPromptOption[];
  files: string[];
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  showComposer: boolean;
  plan: TurnPlan | null;
  debugEntries: DebugEntry[];
  debugOpen: boolean;
  onClearDebug: () => void;
  onCopyDebug: () => void;
  onResizeDebug: (event: MouseEvent<Element>) => void;
  onBackFromDiff: () => void;
  onGoProjects: () => void;
};

type LayoutNodesResult = {
  sidebarNode: ReactNode;
  messagesNode: ReactNode;
  composerNode: ReactNode;
  approvalToastsNode: ReactNode;
  updateToastNode: ReactNode;
  homeNode: ReactNode;
  mainHeaderNode: ReactNode;
  desktopTopbarLeftNode: ReactNode;
  tabletNavNode: ReactNode;
  tabBarNode: ReactNode;
  gitDiffPanelNode: ReactNode;
  gitDiffViewerNode: ReactNode;
  planPanelNode: ReactNode;
  debugPanelNode: ReactNode;
  debugPanelFullNode: ReactNode;
  compactEmptyCodexNode: ReactNode;
  compactEmptyGitNode: ReactNode;
  compactGitBackNode: ReactNode;
};

export function useLayoutNodes(options: LayoutNodesOptions): LayoutNodesResult {
  const activeThreadStatus = options.activeThreadId
    ? options.threadStatusById[options.activeThreadId] ?? null
    : null;

  const sidebarNode = (
    <Sidebar
      workspaces={options.workspaces}
      threadsByWorkspace={options.threadsByWorkspace}
      threadStatusById={options.threadStatusById}
      threadListLoadingByWorkspace={options.threadListLoadingByWorkspace}
      activeWorkspaceId={options.activeWorkspaceId}
      activeThreadId={options.activeThreadId}
      accountRateLimits={options.activeRateLimits}
      onOpenSettings={options.onOpenSettings}
      onOpenDebug={options.onOpenDebug}
      hasDebugAlerts={options.hasDebugAlerts}
      onAddWorkspace={options.onAddWorkspace}
      onSelectHome={options.onSelectHome}
      onSelectWorkspace={options.onSelectWorkspace}
      onConnectWorkspace={options.onConnectWorkspace}
      onAddAgent={options.onAddAgent}
      onAddWorktreeAgent={options.onAddWorktreeAgent}
      onToggleWorkspaceCollapse={options.onToggleWorkspaceCollapse}
      onSelectThread={options.onSelectThread}
      onDeleteThread={options.onDeleteThread}
      onDeleteWorkspace={options.onDeleteWorkspace}
      onDeleteWorktree={options.onDeleteWorktree}
    />
  );

  const messagesNode = (
    <Messages
      items={options.activeItems}
      isThinking={
        options.activeThreadId
          ? options.threadStatusById[options.activeThreadId]?.isProcessing ?? false
          : false
      }
      processingStartedAt={activeThreadStatus?.processingStartedAt ?? null}
      lastDurationMs={activeThreadStatus?.lastDurationMs ?? null}
    />
  );

  const composerNode = options.showComposer ? (
    <Composer
      onSend={options.onSend}
      onStop={options.onStop}
      canStop={options.canStop}
      disabled={options.isReviewing}
      contextUsage={options.activeTokenUsage}
      queuedMessages={options.activeQueue}
      sendLabel={options.isProcessing ? "Queue" : "Send"}
      draftText={options.draftText}
      onDraftChange={options.onDraftChange}
      attachedImages={options.activeImages}
      onPickImages={options.onPickImages}
      onAttachImages={options.onAttachImages}
      onRemoveImage={options.onRemoveImage}
      prefillDraft={options.prefillDraft}
      onPrefillHandled={options.onPrefillHandled}
      insertText={options.insertText}
      onInsertHandled={options.onInsertHandled}
      onEditQueued={options.onEditQueued}
      onDeleteQueued={options.onDeleteQueued}
      models={options.models}
      selectedModelId={options.selectedModelId}
      onSelectModel={options.onSelectModel}
      reasoningOptions={options.reasoningOptions}
      selectedEffort={options.selectedEffort}
      onSelectEffort={options.onSelectEffort}
      accessMode={options.accessMode}
      onSelectAccessMode={options.onSelectAccessMode}
      skills={options.skills}
      prompts={options.prompts}
      files={options.files}
      textareaRef={options.textareaRef}
    />
  ) : null;

  const approvalToastsNode = (
    <ApprovalToasts
      approvals={options.approvals}
      workspaces={options.workspaces}
      onDecision={options.handleApprovalDecision}
    />
  );

  const updateToastNode = (
    <UpdateToast
      state={options.updaterState}
      onUpdate={options.onUpdate}
      onDismiss={options.onDismissUpdate}
    />
  );

  const homeNode = (
    <Home
      onOpenProject={options.onAddWorkspace}
      onAddWorkspace={options.onAddWorkspace}
      latestAgentRuns={options.latestAgentRuns}
      isLoadingLatestAgents={options.isLoadingLatestAgents}
      onSelectThread={options.onSelectHomeThread}
    />
  );

  const mainHeaderNode = options.activeWorkspace ? (
    <MainHeader
      workspace={options.activeWorkspace}
      parentName={options.activeParentWorkspace?.name ?? null}
      worktreeLabel={options.worktreeLabel}
      disableBranchMenu={options.isWorktreeWorkspace}
      parentPath={options.activeParentWorkspace?.path ?? null}
      worktreePath={options.isWorktreeWorkspace ? options.activeWorkspace.path : null}
      branchName={options.branchName}
      branches={options.branches}
      onCheckoutBranch={options.onCheckoutBranch}
      onCreateBranch={options.onCreateBranch}
    />
  ) : null;

  const desktopTopbarLeftNode = (
    <>
      {options.centerMode === "diff" && (
        <button
          className="icon-button back-button"
          onClick={options.onExitDiff}
          aria-label="Back to chat"
        >
          <ArrowLeft aria-hidden />
        </button>
      )}
      {mainHeaderNode}
    </>
  );

  const tabletNavNode = (
    <TabletNav activeTab={options.tabletNavTab} onSelect={options.onSelectTab} />
  );

  const tabBarNode = (
    <TabBar activeTab={options.activeTab} onSelect={options.onSelectTab} />
  );

  const gitDiffPanelNode = (
    <GitDiffPanel
      mode={options.gitPanelMode}
      onModeChange={options.onGitPanelModeChange}
      branchName={options.gitStatus.branchName || "unknown"}
      totalAdditions={options.gitStatus.totalAdditions}
      totalDeletions={options.gitStatus.totalDeletions}
      fileStatus={options.fileStatus}
      error={options.gitStatus.error}
      logError={options.gitLogError}
      logLoading={options.gitLogLoading}
      files={options.gitStatus.files}
      selectedPath={options.selectedDiffPath}
      onSelectFile={options.onSelectDiff}
      logEntries={options.gitLogEntries}
      logTotal={options.gitLogTotal}
      logAhead={options.gitLogAhead}
      logBehind={options.gitLogBehind}
      logAheadEntries={options.gitLogAheadEntries}
      logBehindEntries={options.gitLogBehindEntries}
      logUpstream={options.gitLogUpstream}
      issues={options.gitIssues}
      issuesTotal={options.gitIssuesTotal}
      issuesLoading={options.gitIssuesLoading}
      issuesError={options.gitIssuesError}
      gitRemoteUrl={options.gitRemoteUrl}
    />
  );

  const gitDiffViewerNode = (
    <GitDiffViewer
      diffs={options.gitDiffs}
      selectedPath={options.selectedDiffPath}
      isLoading={options.gitDiffLoading}
      error={options.gitDiffError}
      onLineReference={options.onDiffLineReference}
      onActivePathChange={options.onDiffActivePathChange}
    />
  );

  const planPanelNode = <PlanPanel plan={options.plan} isProcessing={options.isProcessing} />;

  const debugPanelNode = (
    <DebugPanel
      entries={options.debugEntries}
      isOpen={options.debugOpen}
      onClear={options.onClearDebug}
      onCopy={options.onCopyDebug}
      onResizeStart={options.onResizeDebug}
    />
  );

  const debugPanelFullNode = (
    <DebugPanel
      entries={options.debugEntries}
      isOpen
      onClear={options.onClearDebug}
      onCopy={options.onCopyDebug}
      variant="full"
    />
  );

  const compactEmptyCodexNode = (
    <div className="compact-empty">
      <h3>No workspace selected</h3>
      <p>Choose a project to start chatting.</p>
      <button className="ghost" onClick={options.onGoProjects}>
        Go to Projects
      </button>
    </div>
  );

  const compactEmptyGitNode = (
    <div className="compact-empty">
      <h3>No workspace selected</h3>
      <p>Select a project to inspect diffs.</p>
      <button className="ghost" onClick={options.onGoProjects}>
        Go to Projects
      </button>
    </div>
  );

  const compactGitBackNode = (
    <div className="compact-git-back">
      <button onClick={options.onBackFromDiff}>‹ Back</button>
      <span className="workspace-title">Diff</span>
    </div>
  );

  return {
    sidebarNode,
    messagesNode,
    composerNode,
    approvalToastsNode,
    updateToastNode,
    homeNode,
    mainHeaderNode,
    desktopTopbarLeftNode,
    tabletNavNode,
    tabBarNode,
    gitDiffPanelNode,
    gitDiffViewerNode,
    planPanelNode,
    debugPanelNode,
    debugPanelFullNode,
    compactEmptyCodexNode,
    compactEmptyGitNode,
    compactGitBackNode,
  };
}
