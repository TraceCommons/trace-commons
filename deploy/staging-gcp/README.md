# deploy/staging-gcp

Files for the internal staging environment
(`docs/superpowers/specs/2026-10-21-staging-environment-design.md`).

- `cloudbuild.yaml`: builds the staging server binaries.
- `near-ai-rtmr3-allowlist.json`: the RTMR3 and compose-action allowlist
  that `trace-commons-staging-pins sign --allowlist` puts inside the signed
  NEAR AI pin set (owner decision S16, spec 5.2). A staging app earns a
  model key only if that model entry's replayed RTMR3 events carry these
  values and every post-boot compose action is listed here.

  **As committed it is a placeholder that refuses everything**: its values
  are empty and its lists are empty, so no replayed log and no compose
  action matches it, and a pin set signed with it earns no key. The
  operator fills it from the `--review` file of an `observe` run against
  NEAR AI staging, in a reviewed pull request, after checking each value
  (app id, KMS, OS image, key provider, the four progress markers, and
  every compose action's kind, container and image). Every later change is
  a reviewed pull request too, so a new image or compose action fails
  closed until someone has looked at it. The procedure is in
  `docs/operator/staging-pin-signing.md`.
