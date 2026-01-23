# Real Integration Examples

Examples from actual upstream integrations performed on this repository.

## Example 1: Workspace Home Feature (PR #203)

### Context

Integrated the "Workspace Home / Multi-Runs Feature" from upstream PR #203, which adds:
- Workspace home view when no thread is active
- Multi-run orchestration
- AI-generated metadata for runs

### Files Integrated

| Type | File Path | Action |
|------|-----------|--------|
| Utility | `src/utils/caretPosition.ts` | New file |
| Styles | `src/styles/workspace-home.css` | New file |
| Hook | `src/features/workspaces/hooks/useWorkspaceHome.ts` | New file |
| Component | `src/features/workspaces/components/WorkspaceHome.tsx` | New file |
| Integration | `src/App.tsx` | Modified |
| Integration | `src/features/layout/hooks/useLayoutNodes.tsx` | Modified |

### Process

1. **Fetched upstream files** via WebFetch:
   ```
   https://raw.githubusercontent.com/Dimillian/CodexMonitor/main/src/utils/caretPosition.ts
   https://raw.githubusercontent.com/Dimillian/CodexMonitor/main/src/styles/workspace-home.css
   https://raw.githubusercontent.com/Dimillian/CodexMonitor/main/src/features/workspaces/hooks/useWorkspaceHome.ts
   https://raw.githubusercontent.com/Dimillian/CodexMonitor/main/src/features/workspaces/components/WorkspaceHome.tsx
   ```

2. **Copied new files** directly (no branding changes needed in these specific files)

3. **Modified integration files**:
   - Updated `App.tsx` to import and use new hook
   - Updated `useLayoutNodes.tsx` to render WorkspaceHome component

4. **Fixed type mismatches**:
   - Added missing `suggestionsStyle` prop to `ComposerInput.tsx`
   - Fixed callback ordering in `App.tsx` (moved after hook declarations)

5. **Verification**:
   ```bash
   pnpm tsc --noEmit  # TypeScript check
   cargo build        # Rust check
   ```

### Key Learning

The upstream used a different API than initially expected:
- Upstream: `runs`, `draft`, `runMode`, `modelSelections`
- Initial attempt: `drafts`, `activeDraft`, `createDraft`

**Lesson**: Always fetch and read upstream code directly rather than making assumptions.

---

## Example 2: Image Preview Feature (PR #208)

### Context

Integrated binary file preview functionality.

### Branding Impact

No branding changes required - feature was purely additive UI functionality.

### Files Changed

```
src/features/files/components/FilePreview.tsx
src/styles/file-preview.css
```

---

## Example 3: Cherry-Pick UI/UX Improvements

### Context

Batch integration of multiple UI/UX fixes from upstream.

### Commit Reference

```
60a669f feat: cherry-pick upstream UI/UX improvements
```

### Process

1. Identified multiple small UI fixes in upstream
2. Fetched each file
3. Applied changes individually
4. Tested each change

---

## Integration Patterns

### Pattern A: New Feature (Multiple Files)

When integrating a complete new feature:

1. Start with types/interfaces
2. Add backend commands (Rust)
3. Add service layer
4. Add utility functions
5. Add hooks
6. Add styles
7. Add components
8. Wire up in App.tsx

### Pattern B: Bug Fix (Single File)

When integrating a simple fix:

1. Fetch upstream file
2. Identify the specific change
3. Apply change to local file (don't replace entire file)
4. Verify branding intact

### Pattern C: Refactor (Multiple Related Changes)

When integrating a refactor:

1. Understand the full scope of changes
2. Plan integration order
3. Make changes incrementally
4. Test after each increment
5. Verify no regressions

---

## Common Issues Encountered

### Issue 1: Callback Ordering

**Problem**: TypeScript error about using variable before declaration
```typescript
// ERROR: Block-scoped variable 'resetPullRequestSelection' used before declaration
const handleSelectWorkspaceInstance = useCallback(
  () => {
    resetPullRequestSelection();  // ← Used before declared
  },
  [resetPullRequestSelection],
);
```

**Solution**: Move callback definition after the hook that declares the dependency

### Issue 2: Missing Props

**Problem**: Component prop doesn't exist
```typescript
// ERROR: Property 'suggestionsStyle' does not exist on type 'ComposerInputProps'
<ComposerInput suggestionsStyle={style} />
```

**Solution**: Add the missing prop to the component's type definition and implementation

### Issue 3: Import Mismatches

**Problem**: Upstream imports don't match local file structure

**Solution**: Adjust imports to match local paths while keeping functionality

---

## Verification Commands

After any integration:

```bash
# Quick TypeScript check
pnpm tsc --noEmit

# Full Rust build
cd src-tauri && cargo build && cd ..

# Run dev server
pnpm tauri dev

# Search for leaked branding
grep -r "codex" --include="*.ts" --include="*.tsx" src/ | grep -v "// " | grep -v ".md"
```
