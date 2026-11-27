# Restful Toolkit

Spring / JAX-RS / Kotlin / FastAPI / React route indexing for OpenIDEA. Routes come from
plain text scanning (no PSI), so it works in the WASM sandbox.

## React routes (Vite + react-router)

`.tsx`, `.jsx`, `.ts` and `.js` files are scanned for react-router declarations:

- JSX: `<Route path="/admin"><Route path="users" element={<Users />} /></Route>` and `index` routes
- Data routers: `createBrowserRouter([...])` and `useRoutes([...])` objects with nested `children`
- Route config DSL: `@react-router/dev/routes` `route()`, `index()`, `prefix()` and `layout()` calls with nested arrays
- v7 `route(path, module, ...)` entries jump to the module file (`app/routes/...`); if it is missing the jump
  falls back to the config line
- `element`, `Component` and `lazy: () => import(...)` supply the display name
- Paths that are not literals (constants, template strings with `${}`) are kept as written and
  marked unresolved, the same way Spring constants are

React routes copy as paths only (no dev-server origin is inferred), and `Go to Route` opens the
`<Route>` / route-object line (or the v7 route module).

Upgrading the plugin keeps an older but compatible index visible, and clears file signatures so the
next refresh re-scans everything in place instead of showing an empty tree. Component files are not resolved; the jump targets the declaration.

## Search filters

Same-path routes from different files stay apart with a source filter in the Routes search:

- `/playback.ts` — TypeScript sources only (`ts` covers `.ts` and `.tsx`; `js` covers `.js`/`.jsx`)
- `/playback ext:py` — explicit extension filter (`ext:ts`, `ext:tsx`, `ext:py`, `ext:java`, `ext:kt`)
- `/playback fw:react` — framework filter (`react`, `spring`, `jaxrs`, `fastapi`)

Every result carries a source tag (`[tsx] ANY /playback`), and a `.ext` suffix only filters when it
still leaves a hit; otherwise the query is retried literally, so real dotted paths such as
`/api/report.json` keep working.

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
| `excludeTests` | boolean | `true` | Do not index routes from test sources, dependencies (`node_modules/`, `dist/`) or `*.test.*`/`*.spec.*` files |
| `excludeGlobs` | string | see schema | Comma/newline separated globs; filters at index time so tree, search, copy and endpoints agree |
| `projectPrefixFilter` | string | empty | Keep only routes whose file path contains this text |

Test, dependency and build files are filtered while the index is built, and the rebuild
message reports how many were excluded. When the project root has several first-level directories that
produce routes, the tree (and the `Ctrl+Alt+N` picker) prefixes entries with the
sub-project name.

## Commands

- `Restful Toolkit: Rebuild Route Index` — full rebuild (diagnostics in `<plugin dir>/diagnostics.json`)
- `Restful Toolkit: Go to Route` — `Ctrl+Alt+N` (`Meta+Alt+N`), one-step route picker
- `Restful Toolkit: Copy Full URL` / `Copy Query Param` / `Copy RequestBody JSON`
- `Restful Toolkit: Class to JSON` / `Compressed JSON` / `Bulk Value`
- `Restful Toolkit: Send Request` — response lands in the details pane
