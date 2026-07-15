# Vendored frontend assets

Per the frontend stack decision (W§3.5: htmx + minijinja + one vanilla-JS
island), these files are pulled from upstream releases and embedded into
the binary at build time (`include_str!` in `src/web`), so the running
daemon has no runtime dependency on any CDN — matches the single-binary,
self-hosted-appliance story the rest of the project follows.

| File | Source | Version/ref | License |
|---|---|---|---|
| `htmx.min.js` | `github.com/bigskysoftware/htmx`, `dist/htmx.min.js` | tag `v2.0.7` | BSD 2-Clause |
| `htmx-ext-ws.js` | `github.com/bigskysoftware/htmx-extensions`, `src/ws/ws.js` | commit `13582325` (repo has no per-extension tags) | BSD 2-Clause |

To update: re-fetch from the same upstream paths at a newer tag/commit,
verify the new file's `sha256sum` against the release notes/commit if
available, and update the version/ref column above.
