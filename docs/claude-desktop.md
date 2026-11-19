# Claude Desktop Code with Private AI

Claude Desktop support is gated by the default-off `claude_desktop_private_ai`
environment flag. A compatible build and a reviewed enable are required before
setup appears. Flag disable or expiry closes setup and stops new requests while
accepted calls drain; re-enable can restore the same saved connection.

Turn on Private AI and sign in to NEAR AI in Trace Commons. Open Claude
Desktop setup in the Private AI tools list, select the main and background
models, and choose Save. Choose Copy to copy the environment variables.

In Claude Desktop's Code tab, create a **Local** saved environment, select a
folder, and paste the variables. Start a new session using that environment.
Only Code sessions using this saved environment route through this connection.
Do not set `ANTHROPIC_AUTH_TOKEN`. Trace Commons does not modify Claude's private
stores, terminal settings, or subscription settings.

Ready means the dedicated loopback route is available. Connected means its
local ledger contains a successfully completed inference request. A listening
port, token-count failure, rejected request, or cancelled stream does not mark
it Connected. The proxy records the requested alias separately from the served
NEAR AI model; a model's description of itself does not establish routing.

Disconnect stops accepting requests and invalidates the saved environment.
Accepted requests may finish draining. Choose Save and copy the new variables
before using the connection again. Changing models also rotates the environment
capability. A normal daemon restart reuses the same port and capability, provided
Private AI remains on and the same NEAR AI inference key is available. Turning
Private AI off or removing/changing that key revokes the binding; a later
sign-in does not attach the old environment to another account.

The route contains only the NEAR AI backend. Its main and background models are
explicit; background selection must be in the main model's privacy tier or a
more private tier. Caller model, backend, account, and route overrides cannot
select another route. Other inference facades are absent from this listener.
A port conflict, malformed saved state or substituted symlink fails closed.
A model catalogue change that makes the saved context or output budget unsafe
also refuses the old preset; choose Save to generate a new one.

The preset derives its compaction window and answer limit from the current
catalogue. It caps the answer at the smaller advertised output limit or 8,192 tokens,
reserves that cap in the smaller context window, and sets Claude's
absolute `CLAUDE_CODE_AUTO_COMPACT_WINDOW`. Setup refuses models without context
or output limits, or with a safe input window below 100,000 tokens, because
Claude clamps its absolute setting to that minimum. The generic context setting
alone does not override a Claude alias the client already knows. Automatic
server-side permission classification is disabled because the NEAR AI Chat
translation does not implement its safeguards endpoint.

The local ledger remains available to Insights after disconnect. This connection
does not grant transcript collection consent, create a managed CLI session,
or enable request/response body capture. Real Desktop tool calls, subsequent
turns, compaction, side calls and client fallback remain separate validation
requirements. This setup does not claim that every Desktop network request is
private.
