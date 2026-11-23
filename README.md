# Restful Toolkit

Spring / JAX-RS / Kotlin / FastAPI route indexing for OpenIDEA. Routes come from plain
text scanning (no PSI), so it works in the WASM sandbox.

## Settings (Settings › Plugins › Restful Toolkit)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `baseUrl` | string | `http://localhost:8080` | Fallback base URL when no config file gives a port/context-path |
| `requestTimeoutMs` | integer | `30000` | Timeout for *Send Request* |
| `extraHeaders` | string | empty | `Name: Value` lines added to every request |
| `groupBy` | `module` / `controller` | `module` | Tree grouping |
| `activeProfile` | string | empty | Overrides `spring.profiles.active` |
| `maxDepth` | integer | `4` | JSON nesting limit |
| `lineEnding` | `lf` / `crlf` | `crlf` | Bulk Value line ending |
| `skipSerialVersionUid` | boolean | `true` | Leave `serialVersionUID` out |
| `excludeTests` | boolean | `true` | Do not index routes from test sources |
| `excludeGlobs` | string | see schema | Comma/newline separated globs; filters at index time so tree, search, copy and endpoints agree |
| `projectPrefixFilter` | string | empty | Keep only routes whose file path contains this text |

Test files are filtered while the index is built, and the rebuild message reports how
many were excluded. When the project root has several first-level directories that
produce routes, the tree (and the `Ctrl+Alt+N` picker) prefixes entries with the
sub-project name.

## Commands

- `Restful Toolkit: Rebuild Route Index` — full rebuild (diagnostics in `<plugin dir>/diagnostics.json`)
- `Restful Toolkit: Go to Route` — `Ctrl+Alt+N` (`Meta+Alt+N`), one-step route picker
- `Restful Toolkit: Copy Full URL` / `Copy Query Param` / `Copy RequestBody JSON`
- `Restful Toolkit: Class to JSON` / `Compressed JSON` / `Bulk Value`
- `Restful Toolkit: Send Request` — response lands in the details pane
