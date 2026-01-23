# Upstream Sync Workflow

Detailed step-by-step guide for integrating features from upstream CodexMonitor.

## Phase 1: Discovery

### 1.1 Fetch Latest Upstream

```bash
git fetch upstream
```

### 1.2 Check What's New

```bash
# See commits in upstream not in local
git log --oneline HEAD..upstream/main

# See commits in local not in upstream (our additions)
git log --oneline upstream/main..HEAD

# Compare branches
git diff --stat upstream/main
```

### 1.3 View Specific PR

```bash
# View PR details
gh pr view <PR-NUMBER> --repo Dimillian/CodexMonitor

# See PR diff
gh pr diff <PR-NUMBER> --repo Dimillian/CodexMonitor

# List files changed in PR
gh pr diff <PR-NUMBER> --repo Dimillian/CodexMonitor --name-only
```

### 1.4 Identify Files to Integrate

List the files changed in the upstream PR and categorize:

| Category | Action |
|----------|--------|
| New files | Copy directly, apply branding |
| Modified existing | Compare and merge carefully |
| Deleted files | Check if we have local changes |
| Renamed files | Follow the rename, preserve branding |

## Phase 2: File Fetching

### 2.1 Get Upstream File Content

Use WebFetch with raw GitHub URLs:

```
https://raw.githubusercontent.com/Dimillian/CodexMonitor/main/path/to/file
```

For specific commit/PR:
```
https://raw.githubusercontent.com/Dimillian/CodexMonitor/<commit-sha>/path/to/file
```

### 2.2 Read Local File

```bash
# Read the local version of the file
cat path/to/file
```

### 2.3 Compare Files

Create a mental or written diff:
- What's new in upstream?
- What's different in local (branding, customizations)?
- What needs to be preserved from local?

## Phase 3: Integration

### 3.1 New Files

For files that don't exist locally:

1. Fetch upstream content via WebFetch
2. Apply branding transformations (see branding.md)
3. Write to local path using Write tool
4. Verify file is syntactically correct

### 3.2 Modified Files

For files that exist in both:

1. Read local file content
2. Fetch upstream file content
3. Identify upstream additions/changes
4. Merge changes while preserving:
   - Local branding (claude_bin, ClaudeDoctorResult, etc.)
   - Local customizations
   - Import paths specific to this repo
5. Write merged content
6. Verify TypeScript/Rust compiles

### 3.3 Integration Order (Important!)

When integrating multi-file features, follow this order:

1. **Types** (`src/types.ts`) - Add new type definitions first
2. **Backend** (`src-tauri/src/*.rs`) - Rust commands
3. **Services** (`src/services/*.ts`) - Tauri service layer
4. **Utilities** (`src/utils/*.ts`) - Helper functions
5. **Hooks** (`src/features/*/hooks/*.ts`) - React hooks
6. **Styles** (`src/styles/*.css`) - CSS files
7. **Components** (`src/features/*/components/*.tsx`) - React components
8. **Integration** (`src/App.tsx`, etc.) - Wire everything together

This order prevents import errors and makes incremental testing possible.

## Phase 4: Branding Transformations

Apply these transformations to upstream code:

### Variable Names
```
codex_bin → claude_bin
codexBin → claudeBin
CodexDoctorResult → ClaudeDoctorResult
runCodexDoctor → runClaudeDoctor
run_codex_prompt_once → run_claude_prompt_once
```

### Function Names
```
generateCodexMetadata → generateClaudeMetadata (if applicable)
```

### UI Strings
```
"Codex" → "Claude" (in user-facing text)
"CodexMonitor" → "ClaudeCodeMonitor"
```

### Import Paths
Keep local import structure - don't blindly copy upstream imports if paths differ.

## Phase 5: Verification

### 5.1 TypeScript Check

```bash
pnpm tsc --noEmit
```

Fix any type errors before proceeding.

### 5.2 Rust Check

```bash
cd src-tauri && cargo build
```

Fix any Rust compilation errors.

### 5.3 Runtime Test

```bash
pnpm tauri dev
```

Manually test:
- Feature works as expected
- No console errors
- Existing features still work

### 5.4 Edge Cases

Test:
- Empty states
- Error conditions
- State persistence (localStorage, etc.)

## Phase 6: Commit

### 6.1 Stage Changes

```bash
# Stage specific files (preferred)
git add src/path/to/file1.tsx src/path/to/file2.tsx

# Review what's staged
git diff --staged
```

### 6.2 Commit Message Format

```
feat: <description of feature from upstream>

Integrated from upstream CodexMonitor PR #<number>
- <list of key changes>
- <any local modifications made>

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
```

## Troubleshooting

### TypeScript Errors After Integration

1. Check import paths match local structure
2. Verify all new types are exported from types.ts
3. Check for missing dependencies in hooks/components
4. Verify branding transformations are complete

### Rust Errors After Integration

1. Check command is registered in lib.rs
2. Verify function signatures match
3. Check for missing use statements
4. Verify claude.rs exports the new function

### Runtime Errors

1. Check browser console for errors
2. Verify localStorage keys match
3. Check for missing props in components
4. Verify service functions are exported

### Feature Doesn't Work

1. Trace data flow from UI to backend
2. Check if all integration points are connected
3. Verify state management is wired correctly
4. Check for conditional rendering issues
