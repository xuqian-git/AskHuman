# Release notes overrides

Release notes shown on each GitHub Release (and surfaced in-app by the self-update
feature) are generated automatically from Conventional Commits by **git-cliff**
(`cliff.toml`). See `AGENTS.md` → "Commit messages" for how a commit becomes a
changelog line.

## Per-version override

To hand-write (or AI-generate) the notes for a specific version, add a file here:

```
docs/release-notes/v<version>.md   e.g. docs/release-notes/v0.6.0.md
```

If that file exists when `vX.Y.Z` is tagged, the release workflow uses its content
**verbatim** as the release body and skips git-cliff for that version. This is the
hook for bilingual / curated notes. If the file is absent, git-cliff generates the
notes from the commit range.

The file content is plain Markdown (it becomes the GitHub Release body as-is).

## Per-commit overrides (without a file)

Inside a commit message footer you can also:

- `Release-Note: <text>` — replace the subject with `<text>` in the changelog.
- `Release-Note: skip` — exclude that commit from the changelog.
