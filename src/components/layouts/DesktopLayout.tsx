import type { MouseEvent, ReactNode } from "react";
import { MainTopbar } from "../MainTopbar";

type DesktopLayoutProps = {
  sidebarNode: ReactNode;
  updateToastNode: ReactNode;
  approvalToastsNode: ReactNode;
  homeNode: ReactNode;
  showHome: boolean;
  showWorkspace: boolean;
  topbarLeftNode: ReactNode;
  centerMode: "chat" | "diff";
  messagesNode: ReactNode;
  gitDiffViewerNode: ReactNode;
  gitDiffPanelNode: ReactNode;
  planPanelNode: ReactNode;
  composerNode: ReactNode;
  debugPanelNode: ReactNode;
  hasActivePlan: boolean;
  onSidebarResizeStart: (event: MouseEvent<HTMLDivElement>) => void;
  onRightPanelResizeStart: (event: MouseEvent<HTMLDivElement>) => void;
  onPlanPanelResizeStart: (event: MouseEvent<HTMLDivElement>) => void;
};

export function DesktopLayout({
  sidebarNode,
  updateToastNode,
  approvalToastsNode,
  homeNode,
  showHome,
  showWorkspace,
  topbarLeftNode,
  centerMode,
  messagesNode,
  gitDiffViewerNode,
  gitDiffPanelNode,
  planPanelNode,
  composerNode,
  debugPanelNode,
  hasActivePlan,
  onSidebarResizeStart,
  onRightPanelResizeStart,
  onPlanPanelResizeStart,
}: DesktopLayoutProps) {
  return (
    <>
      {sidebarNode}
      <div
        className="sidebar-resizer"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize sidebar"
        onMouseDown={onSidebarResizeStart}
      />

      <section className="main">
        {updateToastNode}
        {showHome && homeNode}

        {showWorkspace && (
          <>
            <MainTopbar leftNode={topbarLeftNode} />
            {approvalToastsNode}
            <div className="content">
              {centerMode === "diff" ? gitDiffViewerNode : messagesNode}
            </div>

            <div
              className="right-panel-resizer"
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize right panel"
              onMouseDown={onRightPanelResizeStart}
            />
            <div className={`right-panel ${hasActivePlan ? "" : "plan-collapsed"}`}>
              <div className="right-panel-top">{gitDiffPanelNode}</div>
              <div
                className="right-panel-divider"
                role="separator"
                aria-orientation="horizontal"
                aria-label="Resize plan panel"
                onMouseDown={onPlanPanelResizeStart}
              />
              <div className="right-panel-bottom">{planPanelNode}</div>
            </div>

            {composerNode}
            {debugPanelNode}
          </>
        )}
      </section>
    </>
  );
}
