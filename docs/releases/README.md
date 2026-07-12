# Release archive

One file per tagged release: `vX.Y.Z.md`, an exact snapshot of
[`RELEASE_NOTES.md`](../../RELEASE_NOTES.md) as it read at the moment
that tag was cut. `RELEASE_NOTES.md` itself always describes only the
*current, unreleased* work — once a version ships, its notes move here
permanently and `RELEASE_NOTES.md` resets for the next cycle.

This directory is empty until the first tag (`v0.1.0`, per
[docs/ROADMAP.md](../ROADMAP.md)) is cut.

## Where to look for what

| Question | File |
|---|---|
| What's changed across every version, at a glance? | [`CHANGELOG.md`](../../CHANGELOG.md) — cumulative, terse, one file |
| What does the *next* release announce? | [`RELEASE_NOTES.md`](../../RELEASE_NOTES.md) — prose, current cycle only |
| What did release `vX.Y.Z` announce, permanently? | `docs/releases/vX.Y.Z.md` — this directory |

## Cutting a release

Order matters here: the tag has to be created while `RELEASE_NOTES.md`
still holds the real content, *before* it gets archived and reset —
`release.yml` reads whatever `RELEASE_NOTES.md` says at the tagged
commit, so tagging after the reset would publish the empty template.

1. Confirm [`RELEASE_NOTES.md`](../../RELEASE_NOTES.md) accurately
   describes what's shipping.
2. In [`CHANGELOG.md`](../../CHANGELOG.md), move the `[Unreleased]`
   section's content under a new `## [vX.Y.Z] - YYYY-MM-DD` heading, and
   add a fresh empty `## [Unreleased]` above it.
3. Commit both, then `git tag vX.Y.Z` and push the tag —
   `.github/workflows/release.yml` picks it up from there (builds,
   publishes the GitHub Release from `RELEASE_NOTES.md` exactly as it
   reads in this commit).
4. *After* tagging, run `scripts/cut-release.sh vX.Y.Z` — copies
   `RELEASE_NOTES.md` to `docs/releases/vX.Y.Z.md` and resets
   `RELEASE_NOTES.md` back to the template for the next cycle. This
   creates a new commit on `main`; it doesn't (and can't) touch the tag
   already pushed in step 3.
5. Commit and push that reset.
