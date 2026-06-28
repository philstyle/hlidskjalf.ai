#[cfg(feature = "backend-postgres")]
use sqlx::postgres::PgPoolOptions;
#[cfg(feature = "backend-postgres")]
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str());

    match command {
        Some("init") => cmd_init().await,
        Some("create-root-token") => cmd_create_root_token().await,
        Some("create-namespace") => cmd_create_namespace(&args).await,
        Some("register-participant") => cmd_register_participant(&args).await,
        Some("help") | Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        _ => {
            print_help();
            Ok(())
        }
    }
}

fn print_help() {
    eprintln!(
        r#"relay-bootstrap — NexusRelay setup and administration tool

USAGE:
    relay-bootstrap <COMMAND>

COMMANDS:
    init                          Run migrations + create root token (first-time setup)
    create-root-token             Generate and store a new root token
    create-namespace <name>       Create a namespace with operator (requires ROOT_TOKEN)
    register-participant <ns> <host> <agent>
                                  Register a participant (requires ADMIN_TOKEN)
    help                          Show this help

ENVIRONMENT:
    DATABASE_URL    Required — database connection string
                    (postgres://… for central; sqlite://… for embedded/standalone)
    ROOT_TOKEN      Required for create-namespace
    ADMIN_TOKEN     Required for register-participant
"#
    );
}

// Postgres arm preserved byte-identical (central-relay bootstrap path unchanged).
#[cfg(feature = "backend-postgres")]
async fn connect_db() -> Result<relay_db::DbPool, Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL environment variable is required")?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;
    Ok(pool)
}

// SQLite arm: nothing-external seed path for the standalone/embedded relay-plugin.
// Reuses the shared connect helper so the same WAL+busy_timeout+foreign_keys
// pragmas the serving binary uses are applied when seeding (so bootstrap writes
// and relay-api reads/writes share identical pragma semantics on the same file).
#[cfg(feature = "backend-sqlite")]
async fn connect_db() -> Result<relay_db::DbPool, Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL environment variable is required")?;
    relay_db::connect::connect(&database_url, 5, 0)
        .await
        .map_err(Into::into)
}

async fn run_migrations(pool: &relay_db::DbPool) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Running migrations...");
    #[cfg(feature = "backend-postgres")]
    sqlx::migrate!("../relay-db/migrations").run(pool).await?;
    #[cfg(feature = "backend-sqlite")]
    sqlx::migrate!("../relay-db/migrations-sqlite")
        .run(pool)
        .await?;
    eprintln!("Migrations complete.");
    Ok(())
}

async fn cmd_init() -> Result<(), Box<dyn std::error::Error>> {
    let pool = connect_db().await?;
    run_migrations(&pool).await?;
    eprintln!();
    create_and_print_root_token(&pool).await
}

async fn cmd_create_root_token() -> Result<(), Box<dyn std::error::Error>> {
    let pool = connect_db().await?;
    create_and_print_root_token(&pool).await
}

async fn create_and_print_root_token(
    pool: &relay_db::DbPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let key = relay_api::bootstrap::mint_root_token(pool).await?;

    eprintln!("Root token created. Save this key — it cannot be recovered:");
    eprintln!();
    println!("{key}");
    eprintln!();
    eprintln!("Use this token to create namespaces:");
    eprintln!("  ROOT_TOKEN={key} relay-bootstrap create-namespace <name>");
    Ok(())
}

async fn cmd_create_namespace(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let name = args
        .get(2)
        .ok_or("Usage: relay-bootstrap create-namespace <name>")?;

    let root_token =
        std::env::var("ROOT_TOKEN").map_err(|_| "ROOT_TOKEN environment variable is required")?;

    // Validate name
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(
            "namespace name must be non-empty and contain only alphanumeric chars and hyphens"
                .into(),
        );
    }
    let name = name.to_lowercase();

    let pool = connect_db().await?;

    // Verify root token
    let prefix = relay_auth::token::extract_key_prefix(&root_token);
    let rows = relay_db::root_tokens::find_root_tokens_by_prefix(&pool, prefix).await?;
    let mut authenticated = false;
    for row in &rows {
        if relay_auth::token::verify_api_key(&root_token, &row.key_hash)
            .map_err(|e| format!("verify error: {e}"))?
        {
            authenticated = true;
            break;
        }
    }
    if !authenticated {
        return Err("invalid root token".into());
    }

    // Create namespace + operator via the shared bootstrap primitive (single
    // source of truth, also used by `relay-api bootstrap-init`).
    let keys = relay_api::bootstrap::create_operator_namespace(&pool, &name).await?;
    let namespace_id = keys.namespace_id;
    let operator_id = keys.operator_id;
    let admin_key = keys.admin_key;
    let operator_key = keys.operator_key;

    eprintln!("Namespace '{name}' created.");
    eprintln!();
    eprintln!("Namespace ID:  {namespace_id}");
    eprintln!("Operator ID:   {operator_id}  (this is the operator's ledger ID)");
    eprintln!();
    eprintln!("Admin key (manages participants in this namespace):");
    println!("ADMIN_KEY={admin_key}");
    eprintln!();
    eprintln!("Operator key (sends/reads messages as the namespace operator):");
    println!("OPERATOR_KEY={operator_key}");
    eprintln!();
    eprintln!("Register a participant:");
    eprintln!(
        "  ADMIN_TOKEN={admin_key} relay-bootstrap register-participant {name} <host> <agent_name>"
    );
    Ok(())
}

async fn cmd_register_participant(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let ns_name = args
        .get(2)
        .ok_or("Usage: relay-bootstrap register-participant <namespace> <host> <agent_name>")?;
    let host = args
        .get(3)
        .ok_or("Usage: relay-bootstrap register-participant <namespace> <host> <agent_name>")?;
    let agent_name = args
        .get(4)
        .ok_or("Usage: relay-bootstrap register-participant <namespace> <host> <agent_name>")?;

    let admin_token =
        std::env::var("ADMIN_TOKEN").map_err(|_| "ADMIN_TOKEN environment variable is required")?;

    let pool = connect_db().await?;

    // Look up namespace
    let namespace = relay_db::namespaces::get_namespace_by_name(&pool, ns_name)
        .await?
        .ok_or_else(|| format!("namespace '{ns_name}' not found"))?;

    // Verify admin token
    let prefix = relay_auth::token::extract_key_prefix(&admin_token);
    if prefix != namespace.admin_key_prefix {
        return Err("admin token does not match this namespace".into());
    }
    if !relay_auth::token::verify_api_key(&admin_token, &namespace.admin_key_hash)
        .map_err(|e| format!("verify error: {e}"))?
    {
        return Err("invalid admin token".into());
    }

    // Generate participant key
    let api_key = relay_auth::token::generate_participant_key();
    let api_hash = relay_auth::token::hash_api_key(&api_key)
        .map_err(|e| format!("failed to hash key: {e}"))?;
    let api_prefix = relay_auth::token::extract_key_prefix(&api_key).to_string();

    let participant_id = relay_db::participants::create_participant(
        &pool,
        namespace.id,
        Some(host),
        Some(agent_name),
        "agent",
        false,
        &api_prefix,
        &api_hash,
        None,
    )
    .await?;

    // Auto-join the namespace default group so a CLI-registered participant can
    // actually message its namespace (groups Slice 1: same-namespace DM requires a
    // shared group; the default group is the backwards-compat backbone). Without
    // this, a CLI-registered agent is stranded — in zero groups and unable to DM
    // anyone. Mirrors the hook on every other registration path (API/invite/operator).
    relay_db::groups::ensure_default_membership(&pool, namespace.id, participant_id).await?;

    let display_name = format!("{ns_name}/{host}/{agent_name}");

    eprintln!("Participant '{display_name}' registered.");
    eprintln!();
    eprintln!("Participant ID:  {participant_id}  (this is the participant's ledger ID)");
    eprintln!();
    eprintln!("API key:");
    println!("API_KEY={api_key}");
    Ok(())
}
