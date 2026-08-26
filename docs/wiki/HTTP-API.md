# HTTP API

Every route the backend serves. There is no authentication on any of them:
this is a LAN tool with no accounts, and the same is already true of submitting
a result. Put it behind whatever your network already does.

Everything is same-origin with the front end, which is why no CORS or
`Timing-Allow-Origin` header appears anywhere — and why none should be added.
See [Engine Contract](Engine-Contract.md#cross-origin).

## Measurement endpoints

These two exist to satisfy `@cloudflare/speedtest`. Their exact behaviour is
contractual — read [Engine Contract](Engine-Contract.md) before changing
either.

| Route | Purpose |
|---|---|
| `GET /__down?bytes=N` | N bytes of throwaway payload. `bytes=0` is the engine's latency ping — there is no separate ping endpoint |
| `POST /__up?bytes=N` | N bytes read and discarded. **The response is withheld until the whole body has been drained** |

Both:

- carry `cache-control: no-store`, without which the browser answers repeat
  requests from cache and the engine measures the cache;
- carry `server-timing: cfRequestDuration;dur=N`, which is the only spelling
  the engine's parser accepts;
- ignore unknown query parameters, because the engine appends its own
  (`during=download` while measuring loaded latency, for example);
- refuse a `bytes` value above `server.max_transfer_bytes`.

## Application endpoints

| Route | Answers with |
|---|---|
| `GET /api/health` | `ok`. Used by the container health check |
| `GET /api/status` | Site name, version, git SHA, active profile, whether history is on, the requesting client's address and what kind of address it is, and the deployment's auto-start default |
| `GET /api/profiles` | The default profile name and every profile the picker may offer: name, description, `nominalBps`, `autoSelectable` |
| `GET /api/profile?name=X` | One full engine configuration. Omitting `name` serves the configured default; an unknown name is a `400`, never a silent fallback |

`GET /api/profile` is where the settings that keep traffic local are pinned —
`logAimApiUrl` and `logMeasurementApiUrl` are always `null`, and the endpoint
URLs are always relative. The packet-loss stage is dropped from the response
when no relay is configured, so the engine does not stall on a connection that
cannot be established.

## History endpoints

All of these degrade quietly when history is disabled: the list endpoints
return `[]`. `POST /api/results` returns `202 Accepted` with `history is
disabled` — the front end posts a result at the end of every run and should not
have to know whether this deployment keeps them. The other two writes return
`404`.

| Route | Answers with |
|---|---|
| `GET /api/history?limit=N&client=X` | Stored runs, newest first. `limit` defaults to 100. `client` accepts an address, `all` (the default), or `mine` — which means the requesting address, so the page can offer "just this machine" without the browser needing to know its own LAN address |
| `GET /api/results/{id}` | One run in full, including its samples, which is what makes a permalink redraw rather than summarise |
| `POST /api/results` | Store a completed run. The front end posts this when a run finishes |
| `POST /api/results/{id}/note` | `{"note": "..."}` — annotate a run, or clear it with an empty string. Capped in characters rather than bytes, so the limit does not depend on which alphabet it is written in |
| `GET /api/clients` | Who has run tests, most recently active first |
| `POST /api/clients/{ip}/name` | `{"name": "..."}` — name a client, or clear the name. The path segment must parse as an IP address; anything else is a `400` |

A run is attributed to an address the server works out for itself, never to one
the request claims — see [Client Identity](Client-Identity.md).

## Metrics

| Route | Answers with |
|---|---|
| `GET /metrics` | Prometheus text format, the most recent run per client |

Off by default. **When disabled it returns 404 from the handler**, rather than
being left unmounted — an unmounted route falls through to the single-page-app
fallback and would answer a scrape with `200 text/html`. See
[History and Metrics](History-and-Metrics.md#prometheus-metrics).

## Static content

Everything else is the built front end: `/` (the test), `/history.html`,
`/result.html?id=N`, `/compare.html?a=N&b=M`, plus the icon and web manifest.
Unmatched paths fall back to the app shell.

The image also serves `/LICENSE` and `/THIRD-PARTY-NOTICES.md`. The front-end
bundle carries a banner comment pointing at the latter, so that link has to
keep resolving — do not move or rename those files.

---

See also: [Engine Contract](Engine-Contract.md) ·
[Architecture](Architecture.md) ·
[History and Metrics](History-and-Metrics.md) ·
[Client Identity](Client-Identity.md)
