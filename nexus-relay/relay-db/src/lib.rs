#[cfg(all(feature = "backend-postgres", feature = "backend-sqlite"))]
compile_error!("backend-postgres and backend-sqlite are mutually exclusive; enable exactly one");
#[cfg(not(any(feature = "backend-postgres", feature = "backend-sqlite")))]
compile_error!("exactly one backend must be enabled: enable backend-postgres or backend-sqlite");

#[cfg(feature = "backend-postgres")]
pub type DbBackend = sqlx::Postgres;
#[cfg(feature = "backend-sqlite")]
pub type DbBackend = sqlx::Sqlite;
pub type DbPool = sqlx::Pool<DbBackend>;

pub mod connect;

pub mod channels;
pub mod groups;
pub mod host_policy;
pub mod invites;
pub mod ledger;
pub mod namespaces;
pub mod pacts;
pub mod participants;
pub mod root_tokens;
#[cfg(feature = "backend-postgres")]
pub mod stats;
