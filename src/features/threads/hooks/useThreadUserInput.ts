import { useCallback } from "react";
import type { Dispatch } from "react";
import type { RequestUserInputRequest, RequestUserInputResponse } from "../../../types";
import { respondToUserInputRequest } from "../../../services/tauri";
import type { ThreadAction } from "./useThreadsReducer";

type UseThreadUserInputOptions = {
  dispatch: Dispatch<ThreadAction>;
};

export function useThreadUserInput({ dispatch }: UseThreadUserInputOptions) {
  const handleUserInputSubmit = useCallback(
    async (request: RequestUserInputRequest, response: RequestUserInputResponse) => {
      const toolUseId = request.params.tool_use_id || String(request.request_id);

      try {
        await respondToUserInputRequest(
          request.workspace_id,
          request.params.thread_id,
          toolUseId,
          response.answers,
        );
      } catch (error) {
        // Log error but still remove the request from UI.
        // The session may have ended (permission denied, turn completed, or process crashed)
        // before the user could submit their response.
        console.error("[useThreadUserInput] Failed to submit user input:", error);
      }
      // Always remove the request from UI, even on error.
      // If the backend rejected it, the turn has already moved on.
      dispatch({
        type: "removeUserInputRequest",
        requestId: request.request_id,
        workspaceId: request.workspace_id,
      });
    },
    [dispatch],
  );

  return { handleUserInputSubmit };
}
