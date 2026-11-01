// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The authenticated `/v1/account/*` surface.
//!
//! [`AccountRoutes`] is the only way a route joins the authenticated account
//! surface. It records every `(method, path)` it registers, so the set of
//! routes behind `account_auth_middleware` can be read back from the router
//! definition itself. axum cannot enumerate a router's routes; this is what
//! lets a test do it anyway.

use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::handler::Handler;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::MethodRouter;

use crate::ingest::app::{AppState, account_auth_middleware};

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
    /// resolves the session and attaches rotated credentials.
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
    // Read only by tests.
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
