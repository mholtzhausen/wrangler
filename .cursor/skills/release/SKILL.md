---
name: release
description: >-
  Cut a wrangler GitHub release: commit pending changes, build the linux x86_64
  tarball, tag, and publish with gh. Use when the user invokes /release, asks to
  ship a release, publish to GitHub, or build and upload a release after a version
  bump.
disable-model-invocation: true
---

# Release (`/release`)

Ship the current version in `Cargo.toml` to GitHub.

## Prerequisites

Run **`/version-bump`** (or bump manually) **before** `/release`:

- `Cargo.toml` `version = "x.y.z"` matches the top `CHANGELOG.md` heading
- `README.md` `WRANGLER_VERSION` pin updated when present
- `Cargo.lock` wrangler package version synced (`cargo check` or `make release`)

If versions disagree, stop and fix before releasing.

## Preflight

```bash
git status -sb
git diff
git log -3 --oneline
grep '^version' Cargo.toml
head -15 CHANGELOG.md
git tag --sort=-version:refname | head -3
```

Confirm:

- On branch `main` (or merge feature branch into `main` first unless user says otherwise)
- No unintended files in the commit (`dist/` is **never** committed)
- `gh auth status` succeeds

Optional but recommended: `make ci` or at minimum `cargo test`.

## Workflow

```md
Task Progress:
- [ ] Step 1: Verify version and changelog alignment
- [ ] Step 2: Stage and commit release changes
- [ ] Step 3: Build release tarball
- [ ] Step 4: Push, tag, and create GitHub release
- [ ] Step 5: Report release URL
```

### Step 1: Verify version

Read version from `Cargo.toml`:

```bash
VERSION=$(grep '^version' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
echo "$VERSION"
```

Top `CHANGELOG.md` section must be `## $VERSION` (hash optional until after commit).

### Step 2: Commit

Stage source and metadata only:

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md README.md src/ scripts/ Makefile
# Add any other modified tracked files that belong to this release; never dist/
```

Use a concise commit message focused on **why**:

```text
Release vX.Y.Z: <one-line summary from changelog>.

<Optional second sentence with main theme.>
```

If nothing to commit (already committed), skip to Step 3.

After commit, add the short hash to the changelog heading if missing:

```bash
HASH=$(git rev-parse --short HEAD)
# ## X.Y.Z  →  ## X.Y.Z (<hash>)
```

If the hash line was updated, commit again:

```text
Fix changelog commit hash for vX.Y.Z.
```

### Step 3: Build

```bash
make release-dist
```

Produces:

```text
dist/wrangler-${VERSION}-linux-x86_64.tar.gz
```

Verify the tarball exists and is non-empty. Build uses `target/release/wrangler` (do not point `CARGO_TARGET_DIR` elsewhere for `make release-dist`).

### Step 4: Push, tag, release

```bash
git push origin main
git tag "v${VERSION}"
git push origin "v${VERSION}"
```

Create the GitHub release (use `gh`):

```bash
gh release create "v${VERSION}" \
  "dist/wrangler-${VERSION}-linux-x86_64.tar.gz" \
  --title "v${VERSION}" \
  --notes "$(cat <<EOF
## Install

\`\`\`bash
curl -fsSL https://raw.githubusercontent.com/mholtzhausen/wrangler/main/scripts/install.sh | bash
\`\`\`

Pin this version:

\`\`\`bash
WRANGLER_VERSION=${VERSION} curl -fsSL https://raw.githubusercontent.com/mholtzhausen/wrangler/main/scripts/install.sh | bash
\`\`\`

<paste Features/Bugfixes bullets from CHANGELOG.md for this version>
EOF
)"
```

If tag already exists locally/remotely, stop and ask the user — do not force-move tags.

### Step 5: Report

Return:

- Version released
- Commit hash(es)
- Tag name (`vX.Y.Z`)
- GitHub release URL
- Tarball filename attached

## Project conventions

| Item | Value |
|------|-------|
| Remote | `origin` → `github.com:mholtzhausen/wrangler` |
| Tag format | `vX.Y.Z` (matches `Cargo.toml` without `v`) |
| Asset | `dist/wrangler-X.Y.Z-linux-x86_64.tar.gz` |
| Install script | `scripts/install.sh` (uses GitHub releases API) |

## Safety rules

- **Never** commit `dist/` or other build artifacts
- **Never** `git push --force` to `main` or move an existing release tag without explicit user request
- **Never** skip hooks (`--no-verify`) unless the user asks
- Do not bump version in `/release`; that is `/version-bump`'s job
- Push only when the user invoked `/release` (shipping implies push)

## Typical sequence

```text
/version-bump patch   # or minor / major
/release
```
