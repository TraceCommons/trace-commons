# Private AI campaign feed

The native macOS Home campaign is a native card populated from HTTPS JSON.
It shows the shared campaign compiled into the contributor immediately, then
refreshes from the channel's `TCPrivateAICampaignURL`. This requires one app
release to enable; later campaign updates do not require an app release.

## Publish a campaign

The default content is
`crates/trace-commons-contributor/src/private_ai_campaign.json`. It is also the
compiled fallback, exposed through `tc_private_ai_campaign_json`; there is no
second copy of its text in Swift. The current campaign is NEAR AI Red and links
to [near.ai/red](https://near.ai/red). It opens that page; it does not change a
managed session's model or privacy mode.

Validate a candidate without uploading:

```bash
python3 scripts/campaigns/publish.py --channel staging --file /path/to/campaign.json --dry-run
```

Publish it using an already authenticated publishing account:

```bash
python3 scripts/campaigns/publish.py --channel staging --file /path/to/campaign.json --account ACCOUNT
```

Use `--channel production` only when a production campaign is approved. This
feature's test campaign was published to staging only. The publisher does not
change the active gcloud account or project and never makes a bucket public.
It targets the existing public app-update buckets, not the server's artifact
or database-storage bucket.

The feeds are:

- Staging: `https://storage.googleapis.com/tc-staging-updates-20261022/staging/private-ai-campaign.json`
- Production: `https://storage.googleapis.com/tracecommons-flatpak/updates/private-ai-campaign.json`

A fresh load of Home checks the feed. Objects have a five-minute public cache
lifetime. Publish a different candidate file to update the existing app; changing
only the compiled default affects future app builds. Keep the published candidate
under version control for review and rollback. To roll back, republish the prior
JSON file. No signing credentials or app notarization are involved in campaign
publication.

## Content contract

```json
{
  "schema_version": 1,
  "id": "near-ai-red-cybersecurity-models",
  "title": "NEAR AI Red: AI that helps with security work",
  "body": "Abliterated DeepSeek V4.1 Flash and Qwen 3.8 answer security questions instead of refusing them. They run in NEAR AI's secure enclaves, so your findings stay private. NEAR AI private.",
  "action_title": "Explore NEAR AI Red",
  "action_url": "https://near.ai/red",
  "symbol": "cpu",
  "image_url": null
}
```

The schema supports a title (100 characters), body (300), action title (40),
and campaign ID (100). Text must be nonempty plain text without markup or
control characters. Destinations must be HTTPS links on exactly `near.ai`.
The native artwork presets are `cpu`, `sparkles`, `bolt`, and `globe`.

Optional `image_url` artwork must be a PNG or JPEG beside the manifest on the
same HTTPS host and directory, at most 2 MiB and 4096 by 4096 pixels. Upload the
image first, then publish the JSON that names it. Use a new image filename when
changing the artwork to avoid cached images. Without usable artwork the native
symbol remains visible. The existing native card gradient and layout remain
under app control.

The client uses anonymous requests with no stored cookies, credentials, or
analytics. It permits no redirects, limits JSON to 16 KiB, and applies five-second
request/resource timeouts. Missing, unsupported, oversized, invalid, and failed
feeds show the compiled fallback silently. No remote HTML, JavaScript, or SVG is
rendered. A cancelled or superseded load cannot replace newer content.
