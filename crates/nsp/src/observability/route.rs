//! Auth-gated `/metrics` route.
//!
//! When [`MetricsAuth::Bearer`] is set, the route accepts only requests whose
//! `Authorization: Bearer <token>` header matches, using constant-time
//! comparison. Otherwise it falls through to the admin JWT middleware.
//!
//! In either case the response is the raw Prometheus text format rendered by
//! [`PrometheusHandle::render`].

use std::sync::Arc;

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use metrics_exporter_prometheus::PrometheusHandle;
use nsp_api::AppState;
use nsp_core::auth;
use secrecy::ExposeSecret;
use subtle::ConstantTimeEq;

use super::MetricsAuth;

/// Mount `/metrics` onto `router`. The handler is auth-gated per `auth`.
///
/// This function is additive: it clones the router, attaches a single route,
/// and does not touch anything else in the app. The API router has already
/// applied its state, so this operates on a stateless [`Router`].
pub fn attach_metrics_route(
    router: Router,
    handle: PrometheusHandle,
    auth: MetricsAuth,
    state: Arc<AppState>,
) -> Router {
    let h = Arc::new(handle);

    match auth {
        MetricsAuth::Bearer(token) => {
            let gate = BearerGate { token };
            router.route(
                "/metrics",
                get({
                    let h = h.clone();
                    move |req: Request| bearer_metrics(h.clone(), gate.clone(), req)
                }),
            )
        }
        MetricsAuth::AdminJwt => {
            let gate = JwtGate { state };
            router.route(
                "/metrics",
                get({
                    let h = h.clone();
                    move || async move { render(&h) }
                })
                .route_layer(middleware::from_fn(
                    move |req: Request, next: Next| {
                        let gate = gate.clone();
                        async move { require_admin_jwt(gate, req, next).await }
                    },
                )),
            )
        }
    }
}

#[derive(Clone)]
struct BearerGate {
    token: Arc<secrecy::SecretString>,
}

async fn bearer_metrics(handle: Arc<PrometheusHandle>, gate: BearerGate, req: Request) -> Response {
    let offered = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let expected = gate.token.expose_secret();
    let ok = match offered {
        Some(t) => t.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() == 1,
        None => false,
    };
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    render(&handle).into_response()
}

#[derive(Clone)]
struct JwtGate {
    state: Arc<AppState>,
}

async fn require_admin_jwt(gate: JwtGate, mut req: Request, next: Next) -> Response {
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Some(raw) = header.and_then(|v| v.strip_prefix("Bearer ")) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match auth::decode_jwt(raw, &gate.state.jwt_key) {
        Ok(decoded) => {
            req.extensions_mut().insert(decoded.claims);
            next.run(req).await
        }
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

fn render(handle: &PrometheusHandle) -> Response {
    let body = handle.render();
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
