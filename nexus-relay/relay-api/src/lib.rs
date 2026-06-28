pub mod bootstrap;
pub mod config;
pub mod error;
pub mod managed;
pub mod routes;
pub mod state;
pub mod telemetry;
pub mod visibility;

pub use relay_db::DbPool;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::response::Html;
use axum::routing::{delete, get, patch, post, put};
use state::AppState;

const DASHBOARD_HTML: &str = include_str!("dashboard.html");
const DASHBOARD_ORG_HTML: &str = include_str!("dashboard-org.html");

pub fn build_router(state: AppState) -> Router {
    let blob_routes = Router::new()
        .route("/blobs", post(routes::blobs::upload_blob))
        .route("/blobs/{sha}", get(routes::blobs::download_blob))
        .layer(DefaultBodyLimit::max(11 * 1024 * 1024));

    let api_routes = Router::new()
        .route("/health", get(routes::health::health))
        .route("/ready", get(routes::health::ready))
        .route("/admin/flush-archive", post(routes::admin::flush_archive))
        .route(
            "/namespaces",
            post(routes::namespaces::create_namespace).get(routes::namespaces::list_namespaces),
        )
        .route(
            "/namespaces/{ns}",
            delete(routes::namespaces::delete_namespace),
        )
        .route(
            "/namespaces/{ns}/gateway-channel",
            patch(routes::namespaces::update_gateway_channel),
        )
        .route(
            "/namespaces/{ns}/hosts/{host}/policy",
            put(routes::participants::update_host_policy),
        )
        .route(
            "/namespaces/{ns}/participants",
            post(routes::participants::register_participant)
                .get(routes::participants::list_participants),
        )
        .route(
            "/namespaces/{ns}/participants/{id}",
            delete(routes::participants::deactivate_participant),
        )
        .route(
            "/namespaces/{ns}/participants/{id}/rotate-key",
            post(routes::participants::rotate_participant_key),
        )
        .route(
            "/namespaces/{ns}/participants/{id}/metadata",
            patch(routes::participants::update_metadata),
        )
        .route(
            "/namespaces/{ns}/participants/{id}/notify-config",
            patch(routes::participants::update_notify_config),
        )
        .route(
            "/namespaces/{ns}/groups",
            post(routes::groups::create_group).get(routes::groups::list_namespace_groups),
        )
        .route(
            "/namespaces/{ns}/groups/{group_id}",
            delete(routes::groups::delete_group),
        )
        .route(
            "/namespaces/{ns}/groups/{group_id}/members",
            post(routes::groups::add_member),
        )
        .route(
            "/namespaces/{ns}/groups/{group_id}/members/{participant_id}",
            delete(routes::groups::remove_member),
        )
        .route("/groups", get(routes::groups::list_all_groups))
        .route(
            "/participants/search",
            get(routes::participants::search_participants),
        )
        .route("/participants/me", get(routes::participants::get_me))
        .route(
            "/participants/me/outbox",
            get(routes::participants::get_my_outbox),
        )
        .route(
            "/participants/me/rotate-key",
            post(routes::participants::rotate_own_key),
        )
        .route(
            "/participants/me/description",
            patch(routes::participants::update_my_description),
        )
        .route(
            "/participants/me/notify-config",
            patch(routes::participants::update_my_notify_config),
        )
        .route(
            "/channels",
            post(routes::channels::create_channel).get(routes::channels::list_channels),
        )
        .route(
            "/channels/{name}/append",
            post(routes::channels::append_to_channel),
        )
        .route("/channels/{name}/read", get(routes::channels::read_channel))
        .route("/channels/{name}/head", get(routes::channels::head_channel))
        .route("/ledger/{ledger_id}/append", post(routes::ledger::append))
        .route("/ledger/{ledger_id}/forward", post(routes::ledger::forward))
        .route("/ledger/{ledger_id}/read", get(routes::ledger::read))
        .route("/ledger/{ledger_id}/head", get(routes::ledger::head))
        .route(
            "/ledger/@{ns}/append",
            post(routes::ledger::append_to_operator),
        )
        .route(
            "/ledger/@{ns}/forward",
            post(routes::ledger::forward_to_operator),
        )
        .route("/ledger/@{ns}/read", get(routes::ledger::read_operator))
        .route("/ledger/@{ns}/head", get(routes::ledger::head_operator))
        .route(
            "/ledger/@{ns}/{host}/{agent_name}/append",
            post(routes::ledger::append_by_address),
        )
        .route(
            "/ledger/@{ns}/{host}/{agent_name}/forward",
            post(routes::ledger::forward_by_address),
        )
        .route(
            "/ledger/@{ns}/{host}/{agent_name}/read",
            get(routes::ledger::read_by_address),
        )
        .route(
            "/ledger/@{ns}/{host}/{agent_name}/head",
            get(routes::ledger::head_by_address),
        )
        ;
    #[cfg(feature = "backend-postgres")]
    let api_routes = api_routes
        .route("/stats", get(routes::stats::get_stats))
        .route("/stats/topology", get(routes::stats::get_topology))
        .route("/stats/activity", get(routes::stats::get_activity));
    let api_routes = api_routes
        .route(
            "/pacts",
            post(routes::pacts::propose_pact).get(routes::pacts::list_pacts),
        )
        .route(
            "/pacts/{id}/approve",
            post(routes::pacts::approve_pact),
        )
        .route(
            "/pacts/{id}/revoke",
            post(routes::pacts::revoke_pact),
        )
        .route("/pacts/partners", get(routes::pacts::list_pact_partners))
        .route(
            "/pacts/verify/{participant_1}/{participant_2}",
            get(routes::pacts::verify_pact),
        )
        .route(
            "/invites",
            post(routes::invites::create_invite).get(routes::invites::list_invites),
        )
        .route(
            "/invites/{id}",
            delete(routes::invites::delete_invite),
        )
        .route("/dashboard", get(|| async { Html(DASHBOARD_HTML) }))
        .route("/dashboard/org", get(|| async { Html(DASHBOARD_ORG_HTML) }));

    // Self-service registration — no auth middleware (invite key in body)
    let register_route = Router::new()
        .route(
            "/namespaces/register",
            post(routes::invites::register_with_invite),
        )
        .with_state(state.clone());

    api_routes
        .merge(blob_routes)
        .merge(register_route)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            inject_extensions,
        ))
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(|req: &axum::http::Request<axum::body::Body>| {
                    let request_id = uuid::Uuid::now_v7();
                    tracing::info_span!(
                        "request",
                        request_id = %request_id,
                        method = %req.method(),
                        path = %req.uri().path(),
                        query = req.uri().query().unwrap_or(""),
                    )
                })
                .on_response(
                    |resp: &axum::http::Response<_>,
                     latency: std::time::Duration,
                     _span: &tracing::Span| {
                        tracing::info!(
                            status = resp.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "response"
                        );
                    },
                ),
        )
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

async fn inject_extensions(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    request.extensions_mut().insert(state.db.clone());
    next.run(request).await
}
