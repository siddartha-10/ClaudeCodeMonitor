import { useMemo } from "react";
import type { PermissionDenial, WorkspaceInfo } from "../../../types";
import type { ApprovalRuleInfo } from "../../../utils/approvalRules";
import { getApprovalRuleInfo } from "../../../utils/approvalRules";

type ApprovalToastsProps = {
  permissionDenials?: PermissionDenial[];
  workspaces: WorkspaceInfo[];
  onPermissionRemember?: (denial: PermissionDenial, ruleInfo: ApprovalRuleInfo) => void;
  onPermissionRetry?: (denial: PermissionDenial, ruleInfo: ApprovalRuleInfo) => void;
  onPermissionDismiss?: (denial: PermissionDenial) => void;
};

export function ApprovalToasts({
  permissionDenials,
  workspaces,
  onPermissionRemember,
  onPermissionRetry,
  onPermissionDismiss,
}: ApprovalToastsProps) {
  const workspaceLabels = useMemo(
    () => new Map(workspaces.map((workspace) => [workspace.id, workspace.name])),
    [workspaces],
  );

  const denials = permissionDenials ?? [];

  if (!denials.length) {
    return null;
  }

  const formatLabel = (value: string) =>
    value
      .replace(/([a-z])([A-Z])/g, "$1 $2")
      .replace(/_/g, " ")
      .trim();

  const renderParamValue = (value: unknown) => {
    if (value === null || value === undefined) {
      return { text: "None", isCode: false };
    }
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      return { text: String(value), isCode: false };
    }
    if (Array.isArray(value)) {
      if (value.every((entry) => ["string", "number", "boolean"].includes(typeof entry))) {
        return { text: value.map(String).join(", "), isCode: false };
      }
      return { text: JSON.stringify(value, null, 2), isCode: true };
    }
    return { text: JSON.stringify(value, null, 2), isCode: true };
  };

  return (
    <div className="approval-toasts" role="region" aria-live="assertive">
      {denials.map((denial) => {
        const workspaceName = workspaceLabels.get(denial.workspace_id);
        const toolInput = denial.tool_input ?? {};
        const inputEntries =
          toolInput && typeof toolInput === "object"
            ? Object.entries(toolInput as Record<string, unknown>)
            : [];
        const ruleInfo = getApprovalRuleInfo({
          ...(toolInput && typeof toolInput === "object"
            ? (toolInput as Record<string, unknown>)
            : {}),
          tool_name: denial.tool_name,
        });
        return (
          <div key={denial.id} className="approval-toast" role="alert">
            <div className="approval-toast-header">
              <div className="approval-toast-title">Permission denied</div>
              {workspaceName ? (
                <div className="approval-toast-workspace">{workspaceName}</div>
              ) : null}
            </div>
            <div className="approval-toast-method">{denial.tool_name}</div>
            <div className="approval-toast-details">
              <div className="approval-toast-detail">
                <div className="approval-toast-detail-label">Notice</div>
                <div className="approval-toast-detail-value">
                  Add to settings.local.json to allow.
                </div>
              </div>
              {inputEntries.length
                ? inputEntries.map(([key, value]) => {
                    const rendered = renderParamValue(value);
                    return (
                      <div key={key} className="approval-toast-detail">
                        <div className="approval-toast-detail-label">
                          {formatLabel(key)}
                        </div>
                        {rendered.isCode ? (
                          <pre className="approval-toast-detail-code">
                            {rendered.text}
                          </pre>
                        ) : (
                          <div className="approval-toast-detail-value">
                            {rendered.text}
                          </div>
                        )}
                      </div>
                    );
                  })
                : null}
            </div>
            <div className="approval-toast-actions">
              <button
                className="secondary"
                onClick={() => onPermissionDismiss?.(denial)}
              >
                Dismiss
              </button>
              {ruleInfo && onPermissionRemember ? (
                <button
                  className="ghost approval-toast-remember"
                  onClick={() => onPermissionRemember(denial, ruleInfo)}
                  title={ruleInfo.label}
                >
                  Always allow
                </button>
              ) : null}
              {ruleInfo && onPermissionRetry ? (
                <button
                  className="primary"
                  onClick={() => onPermissionRetry(denial, ruleInfo)}
                  title={ruleInfo.label}
                >
                  Allow & Retry
                </button>
              ) : null}
            </div>
          </div>
        );
      })}
    </div>
  );
}
