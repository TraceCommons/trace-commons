# NEAR Connect browser dependency

`near-connect-0.11.4.js` is the unmodified `cdn/hot-connect.iife.js` from
`@hot-labs/near-connect` 0.11.4. Upstream identifies the author as HOT Labs and
declares the MIT license in its package metadata. The release tarball and
source revision contain no separate copyright notice or license file.

Sources:

- [Release package metadata](https://registry.npmjs.org/@hot-labs/near-connect/0.11.4)
- [Pinned package.json](https://github.com/azbang/near-connect/blob/da9408fa49d9038f3fa5f57d39b0221df7a66f39/package.json)
- [Pinned browser bundle](https://github.com/azbang/near-connect/blob/da9408fa49d9038f3fa5f57d39b0221df7a66f39/cdn/hot-connect.iife.js)
- [Manifest source](https://github.com/azbang/near-connect/blob/f9c8ae802068a7992ace8e1c2187664429d3cb4b/repository/manifest.json)

The bundle SHA-256 is
`fd2c7d0dee183b6b9b0ea826fb2551f10988e1fb47e480c965b4bc2bb27af03f`.
The npm tarball SHA-512 integrity is
`sha512-exAL1gqysyCHewcMO334YJ3ddTPg3j9EyVM9novz5Jx4fLbG/YSYBlkYMjMNB8E9K/4I7BiemjOw9bQ+h80hEA==`.

`manifest.json` retains the upstream metadata for HOT, Meteor, Nightly, Hana,
NEAR Mobile and NEAR CLI, with a SHA-256 added for each executor. GitHub
executor URLs resolve to verified immutable source revisions. NEAR Mobile's
hosted executor has no versioned URL in the upstream manifest; a changed
response is refused until its replacement is reviewed. Executors
are downloaded anonymously by the native daemon, with redirects refused,
bounded time and size, and an exact digest check before execution. Browser
URLs point to this local verified cache. Cache versions include a fresh page
nonce to prevent execution of code cached by a prior owner of the loopback port.

On 2026-11-06, the NEAR Mobile hosted executor was reviewed again after its
unversioned URL changed. The accepted 95,222-byte response has SHA-256
`df84681559dee638491c158377627a78819fe96fe4be7f8cbad2c756e4e913e7`.
Its mainnet connection and message-signing methods delegate to the external
wallet; this integration still requests no function-call key or transaction.
Its three inline installation/link handlers match the existing CSP hashes.
The connector SDK, wallet permissions and CSP are unchanged. Future upstream
byte changes still fail the exact digest check; they are never accepted
implicitly. The source URL remains unversioned, so this pin does not promise
future availability.

MyNearWallet is excluded following the product owner's retirement report.
Intear generates and stores an application private key during connection.
OKX returns an opaque extension access-key object whose private contents have
not been established. Ledger exposes an additional account-creation flow.
WalletConnect requires separate configuration; Trezu does not advertise
message signing. These adapters remain outside this login integration.

The app requests mainnet message signing without a function-call key or a
transaction. Adapter window/extension permissions remain those in the pinned
manifest. The browser owns temporary connection state; Cloud credentials are
received and stored only by the native daemon. Wallet spending keys remain in
the selected external wallet.

To update, obtain explicit dependency approval, verify npm integrity and the
release's source revision, compare the bundle byte-for-byte, inspect every
retained adapter's connection/signing behavior and permissions, and update its
digest. Run the native authentication and asset tests plus real browser and
wallet verification before both required code-review gates.

CI runs `node scripts/ci/test-near-wallet-browser.cjs` using Node built-ins.
For the SDK/Chromium harness, set `TC_PLAYWRIGHT_MODULE` to an existing
Playwright installation with Chromium, then run
`node scripts/ci/test-near-wallet-sdk.cjs`. Its synthetic loopback signing
blocks external network requests and supplements native cryptographic tests;
valid wallet/Cloud login requires separate verification.
