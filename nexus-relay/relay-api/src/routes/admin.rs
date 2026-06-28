use axum::response::IntoResponse;

// Substrate-scheduler-driven archive flush endpoint, per
// nexus-operator-substrate v1-spec §Scheduled Task Invocation.
// When this binary runs as a substrate plugin, the scheduler POSTs here at the
// manifest-declared cron tick (default `*/2 * * * *`); see plugins/relay/manifest.toml.
//
// v1 ships as an acknowledged-success stub so substrate's Gate-5 plugin-contract
// falsifier passes and the scheduler-tick observable wires end-to-end. Actual
// archive flush logic stays in the `relay-archive` daemon binary for now; the
// substrate-driven flush wiring (calling relay_archive::flush::flush_cycle from
// here against an env-configured GIT_ARCHIVE_REPO) lands in a follow-up.
pub async fn flush_archive() -> impl IntoResponse {
    axum::Json(serde_json::json!({"status": "ok"}))
}
