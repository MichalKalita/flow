# Flow control plane

The admin frontend uses Preact, Tailwind CSS and custom SVG charts. Rust embeds
its generated assets, so production needs no Bun process, frontend server,
external monitoring service or CDN.

## Development

From this directory:

```sh
bun install --frozen-lockfile
bun run format
bun run check
bun test src
bun run build
bunx playwright install chromium
bun run test:e2e
```

Commit `runtime/src/admin-assets/` together with frontend source changes. Restart
the runtime after building: assets are embedded at Rust compile time.

Playwright starts the actual Rust executable with temporary SQLite and telemetry
files, test-only signing credentials, and loopback ports 18081 (HTTP), 11884
(MQTT) and 19091 (admin). These ports must be available. The fixture builds Rust,
forwards shutdown to the child and deletes its temporary data. Tests cover
navigation, graph styling and interaction, mobile layout, JWT generation,
permission-aware HTTP calls, audit JSON decoding, archive log pagination and
admin authentication. Failure screenshots and traces stay in ignored
`test-results/`; use `bunx playwright show-trace <trace.zip>` to investigate.

The admin token is stored in the tab's `sessionStorage`. Application JWTs stay in
page memory and are never written to browser storage.
The HTTP console sends real requests through the runtime's normal permissions.
Logs are queried from bounded memory and rotated JSONL files; audit entries come
from SQLite. Metrics polling pauses when the browser page is hidden.
