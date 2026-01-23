# Branding Transformation Guide

Complete reference for transforming upstream CodexMonitor code to ClaudeCodeMonitor branding.

## Core Principle

This repository is a fork of CodexMonitor that uses "Claude" branding instead of "Codex". When integrating upstream code:

1. **Keep all Claude-specific naming** already in this repo
2. **Transform Codex references** from upstream to Claude
3. **Never blindly replace** - understand context first

## Transformation Tables

### TypeScript/JavaScript

| Upstream (Transform) | This Repo (Keep) |
|---------------------|------------------|
| `codexBin` | `claudeBin` |
| `CodexDoctorResult` | `ClaudeDoctorResult` |
| `runCodexDoctor` | `runClaudeDoctor` |
| `CodexMonitor` | `ClaudeCodeMonitor` |
| `codex` (config keys) | `claude` |

### Rust

| Upstream (Transform) | This Repo (Keep) |
|---------------------|------------------|
| `codex_bin` | `claude_bin` |
| `run_codex_prompt_once` | `run_claude_prompt_once` |
| `codex_` prefix | `claude_` prefix |

### UI Text (User-Facing)

| Upstream (Transform) | This Repo (Keep) |
|---------------------|------------------|
| "Codex" | "Claude" |
| "CodexMonitor" | "ClaudeCodeMonitor" |
| "Ask Codex" | "Ask Claude" |
| "Codex is thinking" | "Claude is thinking" |

## Files with Known Branding

These files contain branding that must be preserved:

### `src/types.ts`
```typescript
// Line ~26: Keep claude_bin
claude_bin?: string | null;

// Line ~99: Keep claudeBin
claudeBin: string | null;

// Line ~145: Keep ClaudeDoctorResult
export type ClaudeDoctorResult = {
  claudeBin: string | null;
```

### `src-tauri/src/claude.rs`
```rust
// Function: run_claude_prompt_once
// All functions use claude_ prefix
```

### `src-tauri/src/lib.rs`
```rust
// Command registrations use claude:: module
claude::run_claude_prompt_once,
claude::generate_commit_message,
```

### `src/services/tauri.ts`
```typescript
// Functions reference claude commands
export async function runClaudeDoctor(): Promise<ClaudeDoctorResult>
```

## Context-Sensitive Transformations

### When to Transform

Transform Codex → Claude when:
- Variable names that would create naming conflicts
- User-facing UI text
- Error messages shown to users
- Documentation strings

### When NOT to Transform

Keep original when:
- Comments referencing upstream repo
- Git history references
- URLs pointing to upstream
- License/attribution text

## Integration Checklist

Before finalizing any integration, verify:

- [ ] All `codex` variable names → `claude`
- [ ] All `Codex` type names → `Claude`
- [ ] All UI strings say "Claude" not "Codex"
- [ ] Rust functions use `claude_` prefix
- [ ] TypeScript types use `Claude` prefix
- [ ] No upstream branding leaked through

## Common Mistakes

### Mistake 1: Over-transforming

```typescript
// WRONG: Transforming comments that reference upstream
// This feature was ported from codex_monitor  ← Keep original reference

// RIGHT: Keep upstream references in comments
// This feature was ported from CodexMonitor PR #203
```

### Mistake 2: Missing nested transformations

```typescript
// WRONG: Transformed outer but missed inner
type ClaudeDoctorResult = {
  codexBin: string;  // ← Missed this!
}

// RIGHT: Transform all related fields
type ClaudeDoctorResult = {
  claudeBin: string;  // ✓ Consistent
}
```

### Mistake 3: Transforming URLs

```typescript
// WRONG: Transforming GitHub URLs
const upstream = "https://github.com/siddartha-10/ClaudeMonitor"  // ← Wrong URL

// RIGHT: Keep upstream reference accurate
const upstream = "https://github.com/Dimillian/CodexMonitor"  // ✓ Accurate
```

## Testing Branding

After integration, search for any leaked upstream branding:

```bash
# Search for codex in TypeScript files
grep -r "codex" --include="*.ts" --include="*.tsx" src/

# Search for Codex in Rust files
grep -r "codex" --include="*.rs" src-tauri/

# These should return NO results (except comments/URLs)
```

## Brand Assets

If integrating UI components with icons/logos:
- Replace any Codex logos with Claude equivalents
- Keep color schemes consistent with Claude branding
- Verify accessibility of branded elements
