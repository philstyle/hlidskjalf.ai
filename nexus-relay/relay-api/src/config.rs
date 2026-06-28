use std::net::SocketAddr;

pub struct AppConfig {
    pub database_url: String,
    pub listen_addr: SocketAddr,
    pub git_blob_repo: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable is required");
        // Bind-address precedence: substrate plugin-contract injection > operator
        // override > default. SUBSTRATE_PLUGIN_HTTP_ADDR is the generic plugin-contract
        // variable nexus-operator-substrate injects from `manifest.ports.http` so the
        // plugin binds the port the host polls (per substrate v1-spec §1 + nexus-relay
        // seq 215 seam decision). LISTEN_ADDR remains the operator override for
        // non-substrate-managed deployments.
        let listen_addr = std::env::var("SUBSTRATE_PLUGIN_HTTP_ADDR")
            .or_else(|_| std::env::var("LISTEN_ADDR"))
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()
            .expect("listen address must be a valid socket address");
        let git_blob_repo = std::env::var("GIT_BLOB_REPO").ok();
        AppConfig {
            database_url,
            listen_addr,
            git_blob_repo,
        }
    }
}
