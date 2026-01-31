# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Context

ClaudeCodeMonitor is a macOS Tauri desktop app that orchestrates multiple Claude Code agents across local workspaces. It is a Claude-branded **fork** of [CodexMonitor](https://github.com/Dimillian/CodexMonitor).

- **origin**: `https://github.com/siddartha-10/ClaudeCodeMonitor.git`
- **upstream**: `https://github.com/Dimillian/CodexMonitor.git`

## Commands

```bash
# Development
npm install               # Install dependencies
npm run tauri:dev         # Full Tauri app (runs doctor check first)
npm run dev               # Vite dev server only (port 1420)

# Validation (run in this order after changes)
npm run lint              # ESLint (.ts, .tsx)
npm run test              # Vitest (single run)
npm run typecheck         # tsc --noEmit (strict mode)

# Rust
cd src-tauri && cargo build    # Rust compilation check
cd src-tauri && cargo test     # Rust tests

# Build
npm run tauri:build       # Production bundle (macOS dmg)
npm run tauri:build:win   # Windows build (uses separate tauri.windows.conf.json)

# Other
npm run test:watch        # Vitest in watch mode
npm run doctor            # Check CMake and other native dependencies
```

**CI runs**: lint → typecheck → test-js → build-macos (all must pass). Rust tests run separately on macOS.

## Critical Branding Rules

**These must ALWAYS be preserved when integrating upstream code:**

| This Repo (Keep) | Upstream (Transform From) |
|------------------|---------------------------|
| `claude_bin` | `codex_bin` |
| `claudeBin` | `codexBin` |
| `ClaudeDoctorResult` | `CodexDoctorResult` |
| `runClaudeDoctor` | `runCodexDoctor` |
| `run_claude_prompt_once` | `run_codex_prompt_once` |
| "Claude" (UI text) | "Codex" (UI text) |

## Architecture

**Tech stack**: React 19 + TypeScript + Vite (frontend), Rust + Tauri 2 (backend), plain CSS (styling).

### How It Works

The backend spawns a `claude app-server` subprocess per workspace, communicating over stdio JSON-RPC. The flow is:
1. `initialize` request → `initialized` notification (must complete before any other requests)
2. Continuous JSON-RPC notification streaming (events, messages, diffs)
3. Server-initiated JSON-RPC requests for approval prompts
4. Frontend receives events via Tauri event system, dispatches through a shared event hub

### Frontend (`src/`)

**Feature-sliced design** — each domain (threads, git, composer, settings, etc.) lives under `src/features/<domain>/` with its own `components/`, `hooks/`, and sometimes `utils/`.

Key architectural rules:
- **`App.tsx`** is the composition root: hook wiring, layout assembly, route/section selection. Keep it lean (~60 line blocks max); extract logic to hooks.
- **Components** are presentational only — props in, UI out. No Tauri IPC calls in components.
- **Hooks** own state, side-effects, and event wiring. Live under `src/features/<domain>/hooks/`.
- **Utils** are pure functions with no React imports. Live in `src/utils/`.
- **Services** (`src/services/tauri.ts`) wrap all Tauri IPC `invoke()` calls. `src/services/events.ts` manages the event hub.
- **Types** shared across features live in `src/types.ts`.
- **Styles**: one CSS file per UI area in `src/styles/`.

No global state library (Redux/Zustand). State management is hooks-based with localStorage for UI preferences (panel sizes, transparency).

### Backend (`src-tauri/src/`)

Key files:
- `lib.rs` — Tauri command registration (150+ commands)
- `backend/claude_cli.rs` — Core subprocess spawning and JSON-RPC handling (largest file)
- `claude.rs` — Thread/message operations
- `workspaces.rs` — Workspace lifecycle (add/remove/connect/persist)
- `git.rs` — Git + GitHub operations (uses libgit2 + `gh` CLI)
- `settings.rs` — App settings persistence
- `claude_config.rs` — Claude `config.toml` feature flag sync
- `dictation.rs` — Whisper speech-to-text (macOS/Linux only, stubbed on Windows)
- `types.rs` — Serde-serializable response types

### Event System (Tauri → React)

One native `listen` per event type, fan-out to React subscribers:
1. Backend emits via `app.emit("event-name", payload)`
2. `src/services/events.ts` defines `createEventHub` — one hub per event, single native listener
3. React hooks subscribe via `useTauriEvent(subscribeXxx, handler)` — never call `listen` directly

### Adding a New Tauri Command

1. Implement the command in `src-tauri/src/` (e.g., in `lib.rs` or a domain module)
2. Register it in the `.invoke_handler()` chain in `lib.rs`
3. Add a wrapper function in `src/services/tauri.ts`
4. Use from hooks/components via the service wrapper

### Adding a New Tauri Event

1. Backend: emit via `app.emit("event-name", payload)` in Rust
2. Frontend: add hub + subscription in `src/services/events.ts`
3. Wire with `useTauriEvent(subscribeMyEvent, handler)` in a hook
4. Update `src/services/events.test.ts` for new subscription helpers

## Common Change Patterns

- **UI layout/styling**: `src/features/*/components/*` and `src/styles/*`
- **App-server event handling**: `src/features/app/hooks/useAppServerEvents.ts`
- **Tauri IPC**: wrappers in `src/services/tauri.ts`, implementation in `src-tauri/src/lib.rs`
- **App settings**: `src/features/settings/hooks/useAppSettings.ts`, `src/features/settings/components/SettingsView.tsx`, `src-tauri/src/settings.rs`
- **Experimental feature toggles**: UI in SettingsView, types in `src/types.ts`, sync to Claude `config.toml` via `src-tauri/src/claude_config.rs`
- **Git behavior**: `src/features/git/hooks/useGitStatus.ts` (polling) + `src-tauri/src/git.rs`
- **Thread rendering**: `src/features/threads/hooks/useThreads.ts`, reducer in `useThreadsReducer.ts`, normalization in `src/utils/threadItems.ts`

## Important Notes

- The window uses `titleBarStyle: "Overlay"` with macOS private APIs for transparency.
- Never send JSON-RPC requests before `initialize`/`initialized` completes — the app-server rejects them.
- Workspaces persist to `workspaces.json`, app settings to `settings.json` (both in app data directory).
- Experimental toggles (`collab`, `steer`, `unified_exec`) sync to `$CLAUDE_HOME/config.toml` (legacy `$CODEX_HOME` supported).
- GitHub features require `gh` CLI installed and authenticated.
- Custom prompts load from `$CLAUDE_HOME/agents` (legacy `$CODEX_HOME` supported).
- TypeScript strict mode is on with `noUnusedLocals` and `noUnusedParameters`. Prefix unused vars with `_`.
- ESLint allows `any` types (`@typescript-eslint/no-explicit-any: off`).
- Tests use Vitest with node environment. Setup file at `src/test/vitest.setup.ts` provides browser API mocks.

## Upstream Sync

Use the `/sync-upstream` skill when integrating features from the upstream CodexMonitor repo.

```
/sync-upstream check              # Check what's new
/sync-upstream 203                # Integrate PR #203
/sync-upstream compare path/file  # Compare specific file
```
