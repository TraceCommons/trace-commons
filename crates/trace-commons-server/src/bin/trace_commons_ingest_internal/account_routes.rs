// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The authenticated `/v1/account/*` surface, and what an unbound account may
//! reach on it (Z2 native passkey identity, slice S1).
//!
//! Two things live here and they are deliberately next to each other:
//!
//! - [`AccountRoutes`], the only way a route joins the authenticated account
//!   surface. It records every `(method, path)` it registers, so the set of
//!   routes behind `account_auth_middleware` can be read back from the router
//!   definition itself. axum cannot enumerate a router's routes; this is what
//!   lets a test do it anyway.
//! - [`UNBOUND_ACCOUNT_ROUTE_POLICY`], which classifies every one of those
//!   routes as reachable or refused for an unbound account. The gate in
//!   `account_auth_middleware` reads it by the request's method and matched
//!   path template; a route missing from it is refused, never reachable.
//!
//! The test `every_authenticated_account_route_is_classified_for_unbound_accounts`
//! compares the two in both directions, so a route added without a
//! classification (or a classification left behind by a removed route) fails
//! CI instead of silently defaulting either way.

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, MatchedPath, Request};
use axum::handler::Handler;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::MethodRouter;

use crate::{AppState, account_auth_middleware};

/// Whether an unbound account's session may reach a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnboundAccess {
    Allowed,
    Refused,
}

/// The label an unbound account's refused request answers with (403).
pub(crate) const ACCOUNT_UNBOUND: &str = "account_unbound";

/// The unbound-account classification of EVERY authenticated account route,
/// as `(method, matched path template, access)`.
///
/// The spec's allowlist
/// (`docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md`,
/// "The unbound gate") is the `Allowed` rows, including the bind routes
/// (`POST /v1/account/near-ai/provision/bind/{start,finish}`, S3), the only
/// way forward. Everything else is `Refused`:
/// invites and the legacy link (an invite must never attach to an account that
/// may close), inference connections, reward reservations (finite offers),
/// NEAR wallet enrolment and payout, merge, adding or removing a passkey, and
/// traces and credit.
///
/// Adding a route to the account surface without a row here fails the
/// classification test. A row whose route is gone fails it too.
pub(crate) const UNBOUND_ACCOUNT_ROUTE_POLICY: &[(&str, &str, UnboundAccess)] = {
    use UnboundAccess::{Allowed, Refused};
    &[
        // The app's state machine and the one way forward.
        ("GET", "/v1/account/binding", Allowed),
        // Z2 S3: connect near.ai. The handlers refuse any account that is not
        // `unbound` (`account_already_bound`), so a bound or closed account
        // that reaches them gets nothing.
        ("POST", "/v1/account/near-ai/provision/bind/start", Allowed),
        ("POST", "/v1/account/near-ai/provision/bind/finish", Allowed),
        // Answers `account_identity_unlinked` for an unbound account.
        ("GET", "/v1/account/contribution-status", Allowed),
        // Cancel is sign-out.
        ("POST", "/v1/account/logout", Allowed),
        ("POST", "/v1/account/sessions/revoke-all", Allowed),
        // See and rename its one passkey.
        ("GET", "/v1/account/passkeys", Allowed),
        ("PATCH", "/v1/account/passkeys/{credential_id}", Allowed),
        // Refused: adding or removing an authenticator.
        ("POST", "/v1/account/passkeys/register/start", Refused),
        ("POST", "/v1/account/passkeys/register/finish", Refused),
        // Z2 S2: one passkey per unbound account.
        (
            "POST",
            "/v1/account/passkeys/native/register/start",
            Refused,
        ),
        (
            "POST",
            "/v1/account/passkeys/native/register/finish",
            Refused,
        ),
        ("DELETE", "/v1/account/passkeys/{credential_id}", Refused),
        // Refused: invites and the legacy invite link.
        ("POST", "/v1/account/invites/redeem", Refused),
        ("POST", "/v1/account/invites/legacy-link/challenge", Refused),
        ("POST", "/v1/account/invites/legacy-link", Refused),
        // Refused: inference connections.
        ("GET", "/v1/account/inference-connection/offers", Refused),
        ("GET", "/v1/account/inference-connection", Refused),
        ("POST", "/v1/account/inference-connection", Refused),
        (
            "DELETE",
            "/v1/account/inference-connection/{connection_id}",
            Refused,
        ),
        // Refused: traces, sessions of traces and credit (empty anyway).
        ("GET", "/v1/account/traces", Refused),
        ("POST", "/v1/account/source-sessions/status", Refused),
        ("GET", "/v1/account/credit-summary", Refused),
        ("GET", "/v1/account/traces/{submission_id}", Refused),
        ("GET", "/v1/account/traces/{submission_id}/content", Refused),
        (
            "GET",
            "/v1/account/traces/{submission_id}/session-detail",
            Refused,
        ),
        (
            "POST",
            "/v1/account/traces/{submission_id}/withdraw",
            Refused,
        ),
        // The pipeline withdrawal (versioned pipeline), an account route like
        // the one above.
        (
            "POST",
            "/v1/contributors/me/pipeline-submissions/{submission_id}/withdraw",
            Refused,
        ),
        (
            "GET",
            "/v1/account/traces/{submission_id}/publication",
            Refused,
        ),
        (
            "PUT",
            "/v1/account/traces/{submission_id}/publication",
            Refused,
        ),
        (
            "DELETE",
            "/v1/account/traces/{submission_id}/publication",
            Refused,
        ),
        // Refused: NEAR wallet enrolment, identities and payout.
        ("POST", "/v1/account/near/enroll/start", Refused),
        ("POST", "/v1/account/near/enroll/finish", Refused),
        ("GET", "/v1/account/near-identities", Refused),
        ("PATCH", "/v1/account/near-identities/{public_key}", Refused),
        (
            "DELETE",
            "/v1/account/near-identities/{public_key}",
            Refused,
        ),
        (
            "PATCH",
            "/v1/account/near-identities/{public_key}/payout",
            Refused,
        ),
        // Refused: merge.
        ("POST", "/v1/account/merge/start", Refused),
        ("POST", "/v1/account/merge/confirm", Refused),
        // Refused: rewards.
        ("GET", "/v1/account/rewards", Refused),
        (
            "GET",
            "/v1/account/reward-reservations/{reservation_id}",
            Refused,
        ),
        (
            "POST",
            "/v1/account/reward-offers/{program_id}/reservations",
            Refused,
        ),
    ]
};

/// What an unbound account's request to `(method, matched_path)` gets. A path
/// the router did not match, or one missing from the policy, is refused.
pub(crate) fn unbound_access(method: &Method, matched_path: Option<&MatchedPath>) -> UnboundAccess {
    let Some(path) = matched_path.map(MatchedPath::as_str) else {
        return UnboundAccess::Refused;
    };
    UNBOUND_ACCOUNT_ROUTE_POLICY
        .iter()
        .find(|(m, p, _)| *m == method.as_str() && *p == path)
        .map(|(_, _, access)| *access)
        .unwrap_or(UnboundAccess::Refused)
}

/// One `(method, path template)` registered on the account surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RegisteredAccountRoute {
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
}

/// Builder for account routes that records each one it registers.
///
/// Registration is per method, so the record cannot disagree with the
/// `MethodRouter` it describes: each helper builds that router itself. The
/// wrapped `Router` is never handed out before [`AccountRoutes::authenticated`]
/// puts it behind `account_auth_middleware`, so there is no way to add an
/// unrecorded route to it.
pub(crate) struct AccountRoutes {
    router: Router<Arc<AppState>>,
    registered: Vec<RegisteredAccountRoute>,
}

impl AccountRoutes {
    pub(crate) fn new() -> Self {
        Self {
            router: Router::new(),
            registered: Vec::new(),
        }
    }

    fn register(
        mut self,
        method: &'static str,
        path: &'static str,
        method_router: MethodRouter<Arc<AppState>>,
    ) -> Self {
        self.registered
            .push(RegisteredAccountRoute { method, path });
        self.router = self.router.route(path, method_router);
        self
    }

    pub(crate) fn get<H, T>(self, path: &'static str, handler: H) -> Self
    where
        H: Handler<T, Arc<AppState>>,
        T: 'static,
    {
        self.register("GET", path, axum::routing::get(handler))
    }

    pub(crate) fn post<H, T>(self, path: &'static str, handler: H) -> Self
    where
        H: Handler<T, Arc<AppState>>,
        T: 'static,
    {
        self.register("POST", path, axum::routing::post(handler))
    }

    /// `POST` with a request body cap tighter than the router default.
    pub(crate) fn post_with_body_limit<H, T>(
        self,
        path: &'static str,
        handler: H,
        limit: usize,
    ) -> Self
    where
        H: Handler<T, Arc<AppState>>,
        T: 'static,
    {
        self.register(
            "POST",
            path,
            axum::routing::post(handler).layer(DefaultBodyLimit::max(limit)),
        )
    }

    pub(crate) fn put<H, T>(self, path: &'static str, handler: H) -> Self
    where
        H: Handler<T, Arc<AppState>>,
        T: 'static,
    {
        self.register("PUT", path, axum::routing::put(handler))
    }

    pub(crate) fn patch<H, T>(self, path: &'static str, handler: H) -> Self
    where
        H: Handler<T, Arc<AppState>>,
        T: 'static,
    {
        self.register("PATCH", path, axum::routing::patch(handler))
    }

    pub(crate) fn delete<H, T>(self, path: &'static str, handler: H) -> Self
    where
        H: Handler<T, Arc<AppState>>,
        T: 'static,
    {
        self.register("DELETE", path, axum::routing::delete(handler))
    }

    /// Fold another group's routes (and their record) into this one.
    pub(crate) fn merge(mut self, other: AccountRoutes) -> Self {
        self.router = self.router.merge(other.router);
        self.registered.extend(other.registered);
        self
    }

    /// A `from_fn` middleware on the routes registered so far, run INSIDE the
    /// account auth middleware. It cannot add routes.
    pub(crate) fn route_layer_fn<F, Fut>(mut self, middleware: F) -> Self
    where
        F: FnMut(Request, Next) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router = self
            .router
            .route_layer(axum::middleware::from_fn(middleware));
        self
    }

    /// Put every registered route behind `account_auth_middleware`, which
    /// resolves the session, enforces the unbound gate, and attaches rotated
    /// credentials.
    pub(crate) fn authenticated(self, state: Arc<AppState>) -> AuthenticatedAccountRoutes {
        AuthenticatedAccountRoutes {
            router: self
                .router
                .route_layer(axum::middleware::from_fn_with_state(
                    state,
                    account_auth_middleware,
                )),
            registered: self.registered,
        }
    }
}

/// Account routes already behind `account_auth_middleware`, with their record.
pub(crate) struct AuthenticatedAccountRoutes {
    router: Router<Arc<AppState>>,
    // Read only by the classification test; production reads the policy.
    #[cfg_attr(not(test), allow(dead_code))]
    registered: Vec<RegisteredAccountRoute>,
}

impl AuthenticatedAccountRoutes {
    pub(crate) fn merge(mut self, other: AuthenticatedAccountRoutes) -> Self {
        self.router = self.router.merge(other.router);
        self.registered.extend(other.registered);
        self
    }

    /// A response mapper OUTSIDE the auth middleware, so it also covers the
    /// middleware's own refusals. It cannot add routes.
    pub(crate) fn map_response<F, Fut>(mut self, map: F) -> Self
    where
        F: FnMut(Response) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.router = self
            .router
            .layer(axum::middleware::map_response::<F, ()>(map));
        self
    }

    #[cfg(test)]
    pub(crate) fn registered(&self) -> &[RegisteredAccountRoute] {
        &self.registered
    }

    pub(crate) fn into_router(self) -> Router<Arc<AppState>> {
        self.router
    }
}
