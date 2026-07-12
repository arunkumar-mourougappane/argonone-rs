#!/usr/bin/env bash
# Archives the current RELEASE_NOTES.md to docs/releases/<version>.md and
# resets RELEASE_NOTES.md to the template for the next cycle.
#
# Run this AFTER tagging and pushing the release tag, never before — see
# docs/releases/README.md for why the ordering matters. This script only
# touches the working tree; it commits nothing and never touches tags.
#
# Usage: scripts/cut-release.sh v0.1.0

set -euo pipefail

if [ $# -ne 1 ]; then
  echo "Usage: $0 <version>  (e.g. $0 v0.1.0)" >&2
  exit 1
fi

VERSION="$1"

if [[ ! "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "ERROR: '$VERSION' doesn't look like vX.Y.Z" >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_NOTES="$REPO_ROOT/RELEASE_NOTES.md"
ARCHIVE_DIR="$REPO_ROOT/docs/releases"
ARCHIVE_FILE="$ARCHIVE_DIR/$VERSION.md"

if [ ! -f "$RELEASE_NOTES" ]; then
  echo "ERROR: $RELEASE_NOTES not found" >&2
  exit 1
fi

if [ -f "$ARCHIVE_FILE" ]; then
  echo "ERROR: $ARCHIVE_FILE already exists — refusing to overwrite" >&2
  exit 1
fi

if ! git -C "$REPO_ROOT" rev-parse "$VERSION" >/dev/null 2>&1; then
  echo "WARNING: tag '$VERSION' doesn't exist locally yet." >&2
  echo "This script is meant to run AFTER tagging (see docs/releases/README.md)." >&2
  read -r -p "Continue anyway? [y/N] " confirm
  [[ "$confirm" =~ ^[Yy]$ ]] || exit 1
fi

mkdir -p "$ARCHIVE_DIR"

# Strip the HTML process-comment block at the top of RELEASE_NOTES.md
# (it documents the process, not the release content) before archiving.
sed '/^<!--$/,/^-->$/d' "$RELEASE_NOTES" | sed '/./,$!d' > "$ARCHIVE_FILE"

cat > "$RELEASE_NOTES" << 'EOF'
<!--
  Current/unreleased release notes only — this is what `gh release create
  --notes-file RELEASE_NOTES.md` publishes to the GitHub Release page
  when a `v*.*.*` tag is pushed (see .github/workflows/release.yml).

  Process for cutting a release (order matters — tag while this file
  still has real content, archive/reset only after):
    1. Make sure this file describes what's actually shipping.
    2. Move the CHANGELOG.md [Unreleased] section to a new [vX.Y.Z] - date
       heading (add a fresh empty [Unreleased] above it).
    3. Commit, tag (`git tag vX.Y.Z`), push the tag — release.yml reads
       this file exactly as it is in that tagged commit.
    4. Only after tagging: run `scripts/cut-release.sh vX.Y.Z` — archives
       this file's content to docs/releases/vX.Y.Z.md and resets it back
       to the template below, in a new commit on main.
  See docs/releases/README.md for the full archive convention.
-->

## Highlights

_Describe what's shipping in the next release._

## What's next

_Point at what's coming after this._
EOF

echo "Archived release notes -> $ARCHIVE_FILE"
echo "Reset RELEASE_NOTES.md to the template."
echo
echo "Next: review/edit RELEASE_NOTES.md's template lines, then commit both"
echo "  git add RELEASE_NOTES.md docs/releases/$VERSION.md"
echo "  git commit -m \"chore: archive $VERSION release notes\""
