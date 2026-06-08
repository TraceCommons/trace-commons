# TraceCommons community site

Static Cloudflare Pages-ready frontend for the public pilot surface at
`https://tracecommons.ai`.

The site renders:

- `GET /api/v1/community/leaderboard`
- `GET /api/v1/community/contributors/{handle}`
- `GET /api/v1/community/analytics/summary`
- `PUT` / `DELETE /api/v1/community/profile`

Cloudflare Pages Functions proxy `/api/v1/community/*` to
`https://ingest.tracecommons.ai/v1/community/*`, keeping browser traffic
same-origin at `tracecommons.ai`.

The operator-curated pilot brief is static Pages data in
[`public/experience.json`](public/experience.json). Update that file to change
the current cohort prompt, milestone targets, and weekly rhythm without
touching the ingest API.

It does not sign device-key requests in the browser. Ironclaw owns local
device keys and upload-claim issuance; the browser profile form accepts only
a short-lived public-attribution Bearer token.

## Local checks

```sh
npm run check
npm run serve
```

Open `http://127.0.0.1:8788`. The app tries the API configured in
[`public/config.js`](public/config.js), then falls back to
[`public/snapshot.json`](public/snapshot.json) if the API is unavailable.
For local API testing, append `?api=http://127.0.0.1:3907`; the deployed
site uses same-origin `/api`.

## Cloudflare Pages

Create a Pages project rooted at this directory:

| Setting | Value |
|---|---|
| Framework preset | None |
| Build command | `npm run check` |
| Build output directory | `public` |
| Custom domain | `tracecommons.ai` |

The repository also includes [`wrangler.toml`](wrangler.toml) for the
existing Cloudflare Pages project `trace-commons-community` with
`pages_build_output_dir = "public"`. For direct upload after authenticating
Wrangler:

```sh
npm run deploy:pages
```

Before deploy, confirm:

- [`public/config.js`](public/config.js) points at
  same-origin `/api`.
- [`public/_worker.js`](public/_worker.js) proxies community API requests to
  `https://ingest.tracecommons.ai`.
- [`public/experience.json`](public/experience.json) has the live cohort
  prompt and milestone targets.
- [`public/_headers`](public/_headers) keeps the CSP and cache headers.
- [`public/_redirects`](public/_redirects) routes SPA paths to `/`.

Do not put invite codes, Bearer tokens, device private keys, raw traces, or
operator-only URLs in `public/`.
