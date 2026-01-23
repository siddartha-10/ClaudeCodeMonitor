---
name: sync-upstream
description: Sync features from upstream CodexMonitor into ClaudeCodeMonitor. Use when integrating PRs, checking for new features, comparing files between repos, or merging upstream changes while maintaining Claude branding.
argument-hint: [PR-number or "check" or "compare path/to/file"]
allowed-tools: Bash(git:*), Bash(gh:*), Read, Grep, Glob, WebFetch, Write, Edit
---

# Upstream Sync Skill for ClaudeCodeMonitor

Synchronize features from upstream [CodexMonitor](https://github.com/Dimillian/CodexMonitor) into this fork while maintaining Claude Code branding.

## Quick Reference

| Command | Purpose |
|---------|---------|
| `/sync-upstream check` | Check what's new in upstream |
| `/sync-upstream 203` | Integrate PR #203 from upstream |
| `/sync-upstream compare src/App.tsx` | Compare specific file with upstream |
| `/sync-upstream status` | Show sync status and divergence |

## Repository Setup

- **Origin**: `https://github.com/siddartha-10/ClaudeCodeMonitor.git` (this fork)
- **Upstream**: `https://github.com/Dimillian/CodexMonitor.git` (source)

## Critical Branding Rules

**NEVER change these - they are intentionally different from upstream:**

| This Repo (Keep) | Upstream (Ignore) |
|------------------|-------------------|
| `claude_bin` | `codex_bin` |
| `claudeBin` | `codexBin` |
| `ClaudeDoctorResult` | `CodexDoctorResult` |
| `runClaudeDoctor` | `runCodexDoctor` |
| `run_claude_prompt_once` | `run_codex_prompt_once` |
| "Claude" in UI text | "Codex" in UI text |
| `ClaudeCodeMonitor` | `CodexMonitor` |

## Workflow Overview

1. **Fetch** - Get latest upstream changes
2. **Identify** - Find new features/PRs to integrate
3. **Fetch Files** - Get upstream file content via raw GitHub URLs
4. **Compare** - Diff upstream vs local files
5. **Integrate** - Copy files, apply branding transforms
6. **Verify** - Build and test

For detailed step-by-step instructions, see [workflow.md](workflow.md).

## File Integration Process

When integrating a file from upstream:

```bash
# 1. Fetch upstream file content
# Use raw GitHub URL: https://raw.githubusercontent.com/Dimillian/CodexMonitor/main/path/to/file

# 2. Compare with local file
# Read both versions and identify differences

# 3. Apply branding transformations
# Replace Codex -> Claude naming where appropriate
# Keep Claude-specific implementations intact

# 4. Preserve local customizations
# Look for Claude-specific features not in upstream
```

For branding transformation details, see [branding.md](branding.md).

## Conflict Resolution

When upstream and local have diverged:

1. **Prefer upstream logic** for new features
2. **Keep local branding** always
3. **Merge carefully** when both have changes to same section
4. **Test thoroughly** after any conflict resolution

## Verification Checklist

After integration, always verify:

```bash
# TypeScript compiles
pnpm tsc --noEmit

# Rust compiles
cd src-tauri && cargo build

# App runs
pnpm tauri dev
```

## Common Tasks

### Check Upstream Status
```bash
git fetch upstream
git log --oneline upstream/main..HEAD  # Local commits not in upstream
git log --oneline HEAD..upstream/main  # Upstream commits not in local
```

### View Upstream PR
```bash
gh pr view 203 --repo Dimillian/CodexMonitor
gh pr diff 203 --repo Dimillian/CodexMonitor
```

### Fetch Upstream File
Use WebFetch with raw GitHub URL:
```
https://raw.githubusercontent.com/Dimillian/CodexMonitor/main/src/path/to/file.tsx
```

## Supporting Files

- [workflow.md](workflow.md) - Detailed step-by-step integration workflow
- [branding.md](branding.md) - Complete branding transformation rules
- [examples.md](examples.md) - Real integration examples from this repo

## When NOT to Use This Skill

- Simple bug fixes that don't involve upstream
- Features unique to this fork
- Documentation-only changes
