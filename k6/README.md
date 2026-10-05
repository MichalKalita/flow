# k6 traffic simulation

HTTP traffic against a running Flow host. The scripts live here so load
generation stays outside `runtime/` and `admin-ui/`.

The default mix is a shopper on the hosted `demo` project: public catalog
reads, signed-in order and device calls, occasional `CreateOrder` /
`SendCommand` mutations, and a lighter sampling of public catalogs on the
other sample projects. JWTs are HS256 tokens for the seeded identities
`idp:u1`, `idp:u2` and `idp:u3`.

The public HTTP router admits 16 in-flight application requests and SQLite
serializes database work. The `traffic` profile stays at 8 VUs with think
time. `stress` ramps past the admission cap so some requests return 503
`overloaded`.

## Prerequisites

Install [k6](https://grafana.com/docs/k6/latest/set-up/install-k6/):

```sh
brew install k6
```

Start the runtime from the repository root (`./start.sh`). Application HTTP
follows `FLOW_HTTP_BIND` (`.env.example` uses `0.0.0.0:80`). Unprivileged
development often uses `127.0.0.1:8080`.

The JWT secret must match `FLOW_JWT_SECRET`. The development default is
`development-key-32-bytes-minimum-123456`.

## Run

k6 resolves the script path from the current working directory. `k6/traffic.js`
only exists at the repository root; from inside this folder that path is a
missing module and k6 prints `moduleSpecifier "k6/traffic.js" couldn't be found`.

From the repository root:

```sh
k6 run ./k6/traffic.js
```

From this folder:

```sh
k6 run traffic.js
```

From any directory, using the wrapper (extra flags are passed to k6):

```sh
./k6/run.sh
./k6/run.sh -e PROFILE=smoke
```

Smoke (1 VU, 20 seconds):

```sh
k6 run -e PROFILE=smoke ./k6/traffic.js
```

Stress (ramps to 24 VUs; 503s are expected):

```sh
k6 run -e PROFILE=stress ./k6/traffic.js
```

Override the target and signing key:

```sh
k6 run -e BASE_URL=http://127.0.0.1:8080 -e JWT_SECRET='your-secret' ./k6/traffic.js
```

## What the mix does

| Weight | Call | Auth |
| --- | --- | --- |
| every iteration | `GET /demo/api/products` | anonymous |
| ~50% | `GET /demo/api/products/{id}/photos` | anonymous |
| ~35% | public catalog on another hosted project | anonymous |
| ~60% | `GET /demo/api/users` and that user's orders | JWT |
| ~40% of users with a device | device status, alerts, sometimes a command | JWT |
| ~15% | `POST /demo/api/orders` (quantity 1) | JWT |

Seeded product stock is small. Once it hits zero, `CreateOrder` returns 400
`invalid_input`. The script counts those as `order_sold_out` and does not
treat them as HTTP failures. Restart with a fresh `data/projects/demo.sqlite`
if you need a clean stock again.

Grouped URL tags keep path parameters out of metric cardinality
(`/demo/api/orders/{orderId}` rather than one series per id).

## Files

- `run.sh` — `cd` to this folder and run `traffic.js`
- `traffic.js` — profiles `smoke`, `traffic`, `stress`
- `jwt.js` — HS256 helper matching the demo issuer and audience
