# Browser authentication

FileNode supports OAuth 2 Authorization Code with PKCE S256 through the operating system's browser. It discovers OAuth/OIDC providers, requests access tokens for JMAP, and never uses ID tokens as API credentials. This is an OAuth resource client; it does not consume ID-token identity claims or claim full OpenID Connect relying-party conformance.

## User workflow

1. Enter the JMAP server and username in account settings. Sign in saves and applies these account changes, safely stopping the previous sync worker first when necessary.
2. Select **Browser / single sign-on** as the sign-in method. Keep the default issuer/client settings for a compatible Stalwart server, or enter the administrator-provided issuer, public client ID, scopes and callback port.
3. Choose **Sign in / Anmelden**. FileNode saves the current form values first, then opens the provider's page in the browser, where its password, MFA and consent rules apply. FileNode never receives the user's login password.
4. The browser returns to a temporary `http://127.0.0.1:<port>/oauth/callback` listener. The listener exists only for that sign-in and closes after success, cancellation or timeout. Return to FileNode after the confirmation page.
5. Later application starts use the saved refresh token. They never open a browser automatically. Expired/revoked authorization requires another explicit sign-in. Pause cancels a pending callback wait.

App-password authentication remains available, with a separate credential entry. Signing in again does not move or reset the sync baseline. The existing account-ID binding still prevents a different browser account from overwriting an established local folder.

The Account page has one **Sign in** action for either method. It sends the current form settings to the worker and waits for the successful save acknowledgement before starting authentication. Configuration errors stop that sequence and are shown in the page; no separate Save button is needed on this page.

## Server requirements

The default requested scopes are `urn:ietf:params:oauth:scope:files offline_access`. The issuer must advertise PKCE S256 and support public clients (`token_endpoint_auth_method=none`), refresh tokens and the requested scopes. Discovery, registration and token endpoints require HTTPS on the configured issuer origin and never follow redirects. When no client ID is set, a compatible provider is asked to register FileNode as a public native application. An installation does not register anything until its user explicitly starts browser sign-in.

Stalwart deployments can advertise Authorization Code, refresh tokens, public clients and PKCE S256 while listing only `openid`, `offline_access`, mail, contacts and calendars scopes. The Stalwart **v0.16.27** source has no files scope in `SUPPORTED_SCOPES`. FileNode rejects its default file-only browser flow when that scope is missing, before client registration or browser launch. It does not silently replace file-only authorization with full-account access. App-password synchronization remains available.

Use a Stalwart version/configuration that advertises and grants file access, or configure an external authorization server whose access tokens Stalwart explicitly trusts for that account. Publishing a discovery URL alone does not establish that trust. No production provider/client/directory settings were changed while implementing this feature, and no live user sign-in has been tested.

References: [Stalwart scopes](https://stalw.art/docs/auth/oauth/scopes/), [public client registration](https://stalw.art/docs/auth/oauth/register-client/), [v0.16.27 scope definitions](https://github.com/stalwartlabs/stalwart/blob/v0.16.27/crates/common/src/auth/oauth/mod.rs), [native-app OAuth](https://www.rfc-editor.org/rfc/rfc8252), [PKCE](https://www.rfc-editor.org/rfc/rfc7636).

## External providers such as authentik

An authentik login for another application does not automatically make its tokens valid for Stalwart. The administrator must configure Stalwart as an OAuth resource server/OIDC directory that validates the intended provider, audiences, claims and permissions, and provisions/maps the account correctly. An existing public mail client already trusted by Stalwart can be reused with an additional desktop callback URI. A separate provider is not required for the callback port. Do not reuse a confidential application's client secret in a desktop client.

Configure the provider application for Authorization Code, S256 PKCE, no client secret, and refresh tokens. A registered redirect can be `http://127.0.0.1:54823/oauth/callback`, with `oauth_redirect_port = 54823` in FileNode. Use only the issuer's exact published URL, an administrator-provided public client ID and scopes that the provider grants and Stalwart accepts. The files scope is the default for conforming servers; external-provider claims/scopes must be designed on the server, not guessed in the desktop client.

The callback port alone does not require a new authentik provider. An existing public provider can list an additional exact redirect URI alongside Bulwark's redirects. Every desktop uses its own loopback interface, so the same fixed port works across different computers without router port forwarding. A busy port is reported as an error; choose another registered port or a carefully constrained loopback redirect expression if needed. Do not replace existing redirect entries.

A confidential provider still requires its client secret for Authorization Code, refresh and device grants; PKCE does not remove that requirement. Reusing that secret in every desktop installation is not supported. Changing a provider used by Stalwart/Bulwark to public needs review of those integrations; FileNode never changes it automatically. If a separate public provider eventually becomes necessary, authentik SSO can still reuse the user's browser session, but Stalwart's issuer/audience trust must also cover its tokens.

authentik can omit `none` from discovery's token authentication-method list even though a provider configured as Public accepts secretless requests. FileNode therefore always attempts PKCE without a secret and lets the provider enforce the client's actual type, instead of rejecting it solely from that list. A manual public client ID is appropriate for authentik; its dynamic registration endpoint can require an administrative registration token, which FileNode does not request or store.

`oauth_issuer` can differ from the JMAP origin only because it is explicit profile configuration. The provider's authorization/token/registration endpoints must share that issuer's origin. An issuer at a path, such as authentik's application issuer, is supported through RFC 8414 discovery with an OpenID configuration fallback. Custom-scheme and web redirect callbacks, device flow, client secrets and embedded login webviews are not implemented.

See [Stalwart external OIDC directories](https://stalw.art/docs/auth/backend/oidc/) for the server-side trust relationship. FileNode does not assume that Stalwart brokers an authentik sign-in.

## Credential and request handling

- Each request uses a cryptographically random 256-bit state and PKCE verifier. The callback validates method, path, host, state, optional issuer and duplicate parameters. An invalid unrelated callback does not consume the real request. Input, response and waiting times are bounded.
- Only access tokens authorize JMAP session, methods, uploads and downloads. Tokens are never placed in URLs, TOML, command-line arguments or logs. The browser receives an authorization URL containing state and a challenge, not access/refresh tokens.
- Access tokens remain in memory. The OS credential entry stores the refresh token plus issuer/client/resource/scopes metadata. The credential key is a bounded SHA-256 identity; the underlying service remains `FileNode` for rename compatibility. OS backend size limits can still reject unusually large credentials; the application reports the persistence error rather than writing secrets to a fallback file.
- Expiring access tokens are refreshed, with refresh-token rotation persisted before use. A 401 invalidates the in-memory access token for refresh on the next reconciliation. A mutation is never blindly replayed because of authentication failure. A revoked grant ends synchronization and requires explicit sign-in.
- Authentication libraries and managed UI strings can retain allocations; complete secret-memory erasure is not claimed. The private loopback listener is protected by state/PKCE, not a security boundary against a compromised local user account.

Tests use disposable loopback servers, never a production account. They cover the RFC PKCE vector, callback rejection, cancellation/port cleanup, dynamic public registration, token exchange, rotation/revocation, missing file scopes, and Basic/Bearer JMAP requests for both FileNode schemas. Live provider configuration, OS keyring backends and interactive sign-in remain acceptance work.
