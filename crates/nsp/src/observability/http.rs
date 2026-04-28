//! Tower middleware that emits `nsp_http_requests_total{method,status,route}`.
//!
//! The route label uses axum's `MatchedPath` extension (e.g.
//! `/api/users/:id`) so that ids and UUIDs do not blow up Prometheus label
//! cardinality. Requests that never match a route (404 on an unknown path)
//! report `route="unmatched"`.

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};

use super::METRIC_HTTP_REQUESTS;

/// `axum::middleware::from_fn` compatible handler.
pub async fn track_http(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_owned();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |m| m.as_str().to_owned());

    let response = next.run(request).await;
    let status = response.status().as_u16().to_string();

    metrics::counter!(
        METRIC_HTTP_REQUESTS,
        "method" => method,
        "status" => status,
        "route" => route,
    )
    .increment(1);

    response
}
