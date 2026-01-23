# ClaudeCodeMonitor

A Tauri + React desktop application for monitoring Claude Code sessions.

## Repository Context

This is a **fork** of [CodexMonitor](https://github.com/Dimillian/CodexMonitor) with Claude branding.

### Git Remotes

- **origin**: `https://github.com/siddartha-10/ClaudeCodeMonitor.git` (this fork)
- **upstream**: `https://github.com/Dimillian/CodexMonitor.git` (source)

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

## Tech Stack

- **Frontend**: React + TypeScript + Vite
- **Backend**: Rust + Tauri
- **Styling**: CSS (no framework)

## Project Structure

```
src/                    # React frontend
├── features/           # Feature-based modules
├── services/           # Tauri service layer
├── styles/             # CSS files
├── utils/              # Utility functions
└── App.tsx             # Main application

src-tauri/              # Rust backend
├── src/
│   ├── lib.rs          # Command registration
│   ├── claude.rs       # Claude-specific commands
│   └── ...
└── Cargo.toml
```

## Common Commands

```bash
# Development
pnpm dev              # Start Vite dev server
pnpm tauri dev        # Start full Tauri app

# Build
pnpm build            # Build frontend
pnpm tauri build      # Build full app

# Type checking
pnpm tsc --noEmit     # TypeScript check
cd src-tauri && cargo build  # Rust check
```

## Upstream Sync

Use the `/sync-upstream` skill when integrating features from CodexMonitor.

```
/sync-upstream check              # Check what's new
/sync-upstream 203                # Integrate PR #203
/sync-upstream compare path/file  # Compare specific file
```

See `.claude/skills/sync-upstream/` for detailed documentation.
