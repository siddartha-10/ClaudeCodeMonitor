# Claude Code CLI Migration Notes

## Goal
Port CodexMonitor to run **Claude Code CLI** while preserving the existing UI/feature set (threads, approvals, prompts, worktrees, git helpers, debug panel, etc.).

## Claude Code CLI Capabilities (from docs)
- **CLI commands**: `claude`, `claude -p`, `claude -c`, `claude -r <session>`, `claude update`, `claude mcp`.
- **Output formats**: `--output-format text|json|stream-json` (print mode).
- **Input format**: `--input-format text|stream-json` (print mode).
- **Session control**: `--resume` / `--continue`, `--session-id`, `--fork-session`, `--no-session-persistence`.
- **Model selection**: `--model` (aliases like `sonnet`, `opus`).
- **Permissions**: `settings.json` with `permissions.allow/ask/deny` (rule syntax for tools).
- **Settings hierarchy**: `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json`, plus managed settings.
- **Config/state roots**: `~/.claude/` (settings + state) and `~/.claude.json` for additional state.

Sources:
- CLI reference: https://code.claude.com/docs/en/cli-reference
- Settings: https://code.claude.com/docs/en/settings
- Setup: https://code.claude.com/docs/en/setup

### Stream‑JSON Print Output (Local CLI sample)
Observed via `claude -p "hello" --output-format stream-json --verbose --no-session-persistence`.
- **`--verbose` is required** for `--output-format stream-json` in print mode.
- Output is newline‑delimited JSON events with top‑level `type`.
- Initial event: `{"type":"system","subtype":"init",...}`
  - Includes `cwd`, `session_id`, `tools`, `mcp_servers`, `model`, `permissionMode`, `claude_code_version`, `agents`, `plugins`, `output_style`.
- Response event: `{"type":"assistant","message":{...},"session_id":...,"uuid":...}`
  - `message` contains `model`, `id`, `type:"message"`, `role:"assistant"`, `content:[{type:"text",text:"..."}]`, `usage`, `stop_reason`.
- Final summary: `{"type":"result","subtype":"success",...}`
  - Includes `result`, `usage`, `total_cost_usd`, `num_turns`, `modelUsage`, `permission_denials`.

## Current CodexMonitor Architecture (High‑Level)
- **Backend** spawns `codex app-server` per workspace and speaks JSON‑RPC over stdio.
- **Frontend** listens to `app-server-event` and interprets events like `turn/started`, `item/agentMessage/delta`, `thread/list`, etc.
- **Settings** include `codexBin`, Codex feature flags (collab/steer/unified_exec) synced to `~/.codex/config.toml`.
- **Prompts** are stored under `$CODEX_HOME/prompts` (or `~/.codex/prompts`).
- **Local usage** scans `~/.codex/sessions/.../*.jsonl` for token usage.

## Codex‑Specific Touchpoints (Audit)
### Backend (Rust)
- **Process spawn + JSON‑RPC:**
  - `src-tauri/src/backend/app_server.rs` (stdio JSON‑RPC bridge, initialize/initialized, emits `codex/*` events)
  - `src-tauri/src/codex.rs` (thread/start/resume/list/archive, turn/start, review/start, model/list, account/rateLimits, skills/list, approval response)
  - `src-tauri/src/lib.rs` (commands registered for Codex APIs)
- **Settings + config:**
  - `src-tauri/src/settings.rs` (sync experimental flags via `codex_config`)
  - `src-tauri/src/codex_config.rs` (reads/writes `~/.codex/config.toml`)
  - `src-tauri/src/codex_home.rs` (resolves `CODEX_HOME` / `.codexmonitor`)
- **Prompts:** `src-tauri/src/prompts.rs` (global prompts at `~/.codex/prompts`)
- **Approval rules:** `src-tauri/src/rules.rs` (Codex `prefix_rule` format)
- **Usage:** `src-tauri/src/local_usage.rs` (reads `~/.codex/sessions/.../*.jsonl`)
- **Remote daemon:** `src-tauri/src/bin/codex_monitor_daemon.rs` (mirrors app‑server behavior)

### Frontend (React)
- **Event parsing:** `src/features/app/hooks/useAppServerEvents.ts` (expects `codex/connected`, `codex/requestApproval/*`)
- **Threads + turns:** `src/features/threads/hooks/useThreads.ts` (assumes Codex JSON‑RPC response shapes)
- **Settings UI:** `src/features/settings/components/SettingsView.tsx` (Codex section + doctor)
- **Settings logic:** `src/features/settings/hooks/useAppSettings.ts` (codex doctor)
- **Workspaces:** `src/features/workspaces/hooks/useWorkspaces.ts` (codex_bin overrides)
- **Models:** `src/features/models/hooks/useModels.ts` (defaults to `gpt-5.2-codex`)
- **Prompts UI:** `src/features/prompts/components/PromptPanel.tsx` (mentions `~/.codex/prompts`)
- **Tabs/labels:** `TabBar.tsx` / `TabletNav.tsx` / `App.tsx` (labels “Codex”)
- **Approval UI:** `ApprovalToasts.tsx` (method prefix `codex/requestApproval`)
- **File link protocol:** `src/utils/remarkFileLinks.ts` (`codex-file:`)

## Key Gaps vs Claude Code CLI
1. **No Codex app‑server**: Claude CLI is not a JSON‑RPC daemon; print mode is **one‑shot**.
2. **Thread listing**: CLI docs mention resume/continue but not a session list API.
3. **Approvals**: Claude permissions are handled via `settings.json` + permission rules; no direct `requestApproval` JSON‑RPC events are documented.
4. **Model list + rate limits**: CLI doesn’t expose a model list/rate‑limit RPC.
5. **Prompts**: Claude uses `CLAUDE.md` and settings-based configuration; no official “prompts directory” equivalent to `~/.codex/prompts`.

## Local `~/.claude` Inspection (Read‑Only)
Observed on disk (no modifications):

### Project/session layout
- `~/.claude/projects/<encoded-workspace-path>/sessions-index.json` is the **session catalog** for a workspace.
- Each session is stored at `~/.claude/projects/<encoded-workspace-path>/<sessionId>.jsonl`.
- Some `.jsonl` files are **empty (0 bytes)**.
- Subagent logs live at `~/.claude/projects/<encoded-workspace-path>/<sessionId>/subagents/agent-<id>.jsonl`.

### `sessions-index.json` entry shape
- `sessionId`, `fullPath`, `fileMtime`, `firstPrompt`, `messageCount`.
- `created`, `modified`, `gitBranch`, `projectPath`, `isSidechain`.

### Session JSONL event schema (observed)
Common fields:
- `type` (e.g. `user`, `assistant`, `queue-operation`, `file-history-snapshot`)
- `uuid`, `parentUuid`, `sessionId`, `timestamp`, `cwd`, `version`, `gitBranch`, `userType`, `isSidechain`

Message events:
- `user` / `assistant` entries contain `message.role` and `message.content` (array of text/tool blocks).
- Tool calls appear as `assistant` messages with `content: [{"type":"tool_use", ...}]`.
- Tool results appear as `user` messages with `content: [{"type":"tool_result", ...}]` and a top‑level `toolUseResult` payload.

Other event types:
- `queue-operation` (enqueue/dequeue with `operation` and `content`).
- `file-history-snapshot` (captures `trackedFileBackups`).

### Other state
- `~/.claude/history.jsonl` stores prompt/command history with `display`, `timestamp`, and `project`.
- `~/.claude/settings.json` uses the `claude-code-settings.json` schema (e.g., `statusLine`, `alwaysThinkingEnabled`, `enabledPlugins`).
- `~/.claude/agents/*.md` defines custom agents via YAML frontmatter (`name`, `description`, `tools`, `color`) plus a markdown prompt body.

## Migration Strategy (Likely)
### Option A: **Claude CLI Shim (Preferred for minimal UI changes)**
Build a Rust “shim” that mimics the current Codex app‑server JSON‑RPC API, but internally:
- Spawns `claude -p --output-format stream-json` for each turn.
- Manages session IDs with `--session-id` or `--resume`.
- Persists and lists sessions by reading Claude’s session files on disk (likely `~/.claude/sessions` — verify format).
- Emits synthetic events that match current UI expectations (`turn/started`, `item/*`, `turn/completed`, etc.).

### Thread Mapping (Planned)
- **thread/list**: read `~/.claude/projects/<encoded>/sessions-index.json`, filter entries by `projectPath == workspace.path`, sort by `modified` desc, return `data` with `{ id: sessionId, cwd: projectPath, preview: firstPrompt, updatedAt: modified }` and cursor as offset string.
- **thread/resume**: parse session JSONL to build a single `thread` with one synthetic turn:
  - `user` entries → `items[{ type: "userMessage", id: uuid, content: message.content }]`.
  - `assistant` entries → `items[{ type: "agentMessage", id: uuid, text: joined text blocks }]`.
  - `tool_use` blocks → `items[{ type: "mcpToolCall", id: tool_use.id, server: name, arguments: input, status }]`.
  - `tool_result` blocks → attach `result` + `status:"completed"` to matching `tool_use` item (by `tool_use_id`).

### Option B: **Rework frontend to a new Claude event model**
Replace JSON‑RPC assumptions in UI hooks and update to Claude’s stream‑json output format. More invasive; higher UI changes.

## Claude‑Specific Mappings (Initial)
- **Binary setting**: replace `codexBin`/`codex_bin` with `claudeBin` and check `claude --version` / `claude doctor`.
- **Session storage**: use `~/.claude/projects/<encoded-path>/sessions-index.json` plus per‑session JSONL files.
- **Approvals**: map to Claude permission rules in `.claude/settings.json` or `.claude/settings.local.json`.
- **Experimental flags**: replace Codex feature flags with Claude CLI equivalents (if any), otherwise remove from UI.
- **Prompts**: map to `CLAUDE.md` and/or `.claude` configuration; decide how to handle existing prompt panel.
- **Models**: replace default model with Claude aliases (e.g., `sonnet`, `opus`) and allow custom model IDs.
- **Events**: replace `codex/*` event names; either keep to minimize UI changes or introduce `claude/*`.

## Risks / Unknowns (Need to Validate)
- **Session file format** for listing/resume/archiving threads (not documented in official CLI reference).
- **Streaming event schema** for `--output-format stream-json` (needed for item/tool/assistant deltas).
- **Tool/approval prompts**: how to translate Claude permission prompts into existing approval UI.

### Permissions/Approvals (Observed)
- No explicit approval events observed in session JSONL; permissions appear to be enforced via `settings.json` rules.
- If approvals must be surfaced in‑app, likely need to preconfigure `permissions.ask/allow/deny` and treat missing rules as a UI‑side warning rather than a runtime event.

## Implementation Targets (when coding starts)
1. Backend: replace `app_server` and `codex.rs` flows with a Claude CLI shim.
2. Settings: update `codex_*` settings, config, and doctor flow to Claude.
3. Prompts + rules: map to Claude config (`~/.claude` + `settings.json` rules).
4. Frontend: rename Codex labels, update model defaults, adapt approval parsing/event names.
5. Remote daemon: mirror the new Claude shim in `codex_monitor_daemon`.

## Validation (per project rules)
- `npm run lint`
- `npm run test` (required if thread/settings/shared utils touched)
- `npm run typecheck`
