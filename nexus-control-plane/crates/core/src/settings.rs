use crate::db::DbState;
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    Live,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingType {
    String,
    Bool,
    U64,
    Color,
}

#[derive(Clone, Copy, Debug)]
pub struct SettingSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub section: &'static str,
    pub kind: SettingKind,
    pub value_type: SettingType,
    pub secret: bool,
    pub default_value: &'static str,
    pub env_keys: &'static [&'static str],
    pub env_wins: bool,
    pub writable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SettingView {
    pub key: String,
    pub label: String,
    pub section: String,
    pub value: Option<String>,
    pub kind: SettingKind,
    pub value_type: SettingType,
    pub secret: bool,
    pub is_set: bool,
    pub writable: bool,
    pub env_wins: bool,
}

pub type SharedSettings = Arc<RwLock<SettingsStore>>;

#[derive(Clone)]
pub struct SettingsStore {
    db: DbState,
    values: HashMap<String, String>,
}

const EMPTY_ENVS: &[&str] = &[];
const OWNER_ENVS: &[&str] = &["NCC_OWNER_EMAILS", "NCC_OWNER_EMAIL"];

pub const SETTINGS_CATALOG: &[SettingSpec] = &[
    SettingSpec {
        key: "user_name",
        label: "User name",
        section: "Profile",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: EMPTY_ENVS,
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "github_org",
        label: "GitHub org",
        section: "Workspace",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: &["NCC_GITHUB_ORG"],
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "workspace_root",
        label: "Workspace root",
        section: "Workspace",
        kind: SettingKind::Restart,
        value_type: SettingType::String,
        secret: false,
        default_value: "/workspaces",
        env_keys: &["NCC_WORKSPACE_ROOT"],
        env_wins: true,
        writable: true,
    },
    SettingSpec {
        key: "layout_mode",
        label: "Layout mode",
        section: "Interface",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "split",
        env_keys: EMPTY_ENVS,
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "default_lane_id",
        label: "Default lane",
        section: "Workspace",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: EMPTY_ENVS,
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "owner_emails",
        label: "Owner emails",
        section: "Access",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: OWNER_ENVS,
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "admin_emails",
        label: "Admin emails",
        section: "Access",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: &["NCC_ADMIN_EMAILS"],
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "slack_channel",
        label: "Slack channel",
        section: "Slack",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: &["SLACK_CHANNEL"],
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "slack_idle_enabled",
        label: "Slack idle notifications",
        section: "Slack",
        kind: SettingKind::Live,
        value_type: SettingType::Bool,
        secret: false,
        default_value: "true",
        env_keys: &["SLACK_IDLE_ENABLED"],
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "slack_bot_token",
        label: "Slack bot token",
        section: "Slack",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: true,
        default_value: "",
        env_keys: &["SLACK_BOT_TOKEN"],
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "ncc_port",
        label: "NCC port",
        section: "Instance",
        kind: SettingKind::Restart,
        value_type: SettingType::U64,
        secret: false,
        default_value: "4242",
        env_keys: &["NCC_PORT"],
        env_wins: true,
        writable: false,
    },
    SettingSpec {
        key: "ncc_data_dir",
        label: "NCC data dir",
        section: "Instance",
        kind: SettingKind::Restart,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: &["NCC_DATA_DIR"],
        env_wins: true,
        writable: false,
    },
    SettingSpec {
        key: "ncc_name",
        label: "NCC name",
        section: "Relay",
        kind: SettingKind::Restart,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: &["NCC_NAME"],
        env_wins: true,
        writable: true,
    },
    SettingSpec {
        key: "relay_namespace",
        label: "Relay namespace",
        section: "Relay",
        kind: SettingKind::Restart,
        value_type: SettingType::String,
        secret: false,
        default_value: "",
        env_keys: &["RELAY_NAMESPACE"],
        env_wins: true,
        writable: true,
    },
    SettingSpec {
        key: "relay_admin_key",
        label: "Relay admin key",
        section: "Relay",
        kind: SettingKind::Restart,
        value_type: SettingType::String,
        secret: true,
        default_value: "",
        env_keys: &["RELAY_ADMIN_KEY"],
        env_wins: true,
        writable: true,
    },
    SettingSpec {
        key: "ncc_bootstrap_token",
        label: "Bootstrap token",
        section: "Access",
        kind: SettingKind::Restart,
        value_type: SettingType::String,
        secret: true,
        default_value: "",
        env_keys: &["NCC_BOOTSTRAP_TOKEN"],
        env_wins: true,
        writable: true,
    },
    // --- Identity: name + color this NCC like an iTerm tab (cosmetic; distinct from
    //     NCC_NAME, the relay host identity). Live + writable; clients read on load. ---
    SettingSpec {
        key: "ncc_display_name",
        label: "NCC name",
        section: "Identity",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "Nexus Command Center",
        env_keys: EMPTY_ENVS,
        env_wins: false,
        writable: true,
    },
    SettingSpec {
        key: "ncc_accent_color",
        label: "NCC accent color",
        section: "Identity",
        kind: SettingKind::Live,
        value_type: SettingType::Color,
        secret: false,
        default_value: "#3B82F6",
        env_keys: EMPTY_ENVS,
        env_wins: false,
        writable: true,
    },
    // The relay "shoulder tap" message NCC injects into an idle session on delivery.
    // Live + writable so phrasings can be tested without a restart; the put handler
    // write-throughs to the tap-template file the delivery path reads. {count} → message count.
    SettingSpec {
        key: "relay_tap_template",
        label: "Relay shoulder-tap message",
        section: "Relay",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: crate::relay::DEFAULT_SINGLE_TAP,
        env_keys: EMPTY_ENVS,
        env_wins: false,
        writable: true,
    },
    // The command launched in a session when the operator hits the "Claude" action.
    // Live + writable so the launch flags/settings can change at runtime. The command runs
    // in the session's shell, so $(pwd) expands to the workspace (per-card memory dir).
    SettingSpec {
        key: "claude_launch_command",
        label: "Claude launch command",
        section: "Sessions",
        kind: SettingKind::Live,
        value_type: SettingType::String,
        secret: false,
        default_value: "claude --dangerously-skip-permissions --settings \"{\\\"autoMemoryDirectory\\\": \\\"$(pwd)/memory\\\"}\"",
        env_keys: EMPTY_ENVS,
        env_wins: false,
        writable: true,
    },
];

pub fn catalog() -> &'static [SettingSpec] {
    SETTINGS_CATALOG
}

pub fn spec_for(key: &str) -> Option<&'static SettingSpec> {
    SETTINGS_CATALOG.iter().find(|spec| spec.key == key)
}

pub fn create_shared(db: DbState) -> Result<SharedSettings, String> {
    seed_catalog_defaults(&db)?;
    let values = load_values(&db)?;
    Ok(Arc::new(RwLock::new(SettingsStore { db, values })))
}

fn load_values(db: &DbState) -> Result<HashMap<String, String>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut values = HashMap::new();
    for row in rows {
        let (key, value) = row.map_err(|e| e.to_string())?;
        values.insert(key, value);
    }
    Ok(values)
}

fn seed_catalog_defaults(db: &DbState) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    for spec in SETTINGS_CATALOG {
        let value = first_env(spec).unwrap_or_else(|| spec.default_value.to_string());
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![spec.key, value],
        )
        .map_err(|e| format!("Failed to seed setting '{}': {}", spec.key, e))?;
    }
    Ok(())
}

fn first_env(spec: &SettingSpec) -> Option<String> {
    spec.env_keys
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
}

fn validate_value(spec: &SettingSpec, value: &str) -> Result<(), String> {
    match spec.value_type {
        SettingType::String => Ok(()),
        SettingType::Bool => parse_bool(value)
            .map(|_| ())
            .ok_or_else(|| format!("Setting '{}' expects true/false", spec.key)),
        SettingType::U64 => value
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| format!("Setting '{}' expects a positive integer", spec.key)),
        SettingType::Color => {
            let v = value.trim();
            let ok = v.len() == 7
                && v.starts_with('#')
                && v[1..].chars().all(|c| c.is_ascii_hexdigit());
            if ok {
                Ok(())
            } else {
                Err(format!("Setting '{}' expects a hex color like #3B82F6", spec.key))
            }
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl SettingsStore {
    pub fn get_str(&self, key: &str) -> Option<String> {
        let spec = spec_for(key)?;
        if spec.env_wins {
            if let Some(value) = first_env(spec) {
                return Some(value);
            }
        }
        self.values
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .or_else(|| first_env(spec))
            .or_else(|| {
                if spec.default_value.is_empty() {
                    None
                } else {
                    Some(spec.default_value.to_string())
                }
            })
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get_str(key).and_then(|value| parse_bool(&value))
    }

    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get_str(key)
            .and_then(|value| value.parse::<u64>().ok())
    }

    pub fn view(&self, spec: &SettingSpec) -> SettingView {
        let active_value = self.get_str(spec.key);
        let raw_value = self.values.get(spec.key).cloned();
        let is_set = active_value
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
            || raw_value.as_ref().map(|v| !v.is_empty()).unwrap_or(false);

        SettingView {
            key: spec.key.to_string(),
            label: spec.label.to_string(),
            section: spec.section.to_string(),
            value: if spec.secret { None } else { active_value },
            kind: spec.kind,
            value_type: spec.value_type,
            secret: spec.secret,
            is_set,
            writable: spec.writable,
            env_wins: spec.env_wins && first_env(spec).is_some(),
        }
    }

    pub fn list_views(&self) -> Vec<SettingView> {
        SETTINGS_CATALOG
            .iter()
            .map(|spec| self.view(spec))
            .collect()
    }

    pub fn set(&mut self, key: &str, value: &str) -> Result<SettingView, String> {
        let spec = spec_for(key).ok_or_else(|| format!("Setting '{}' is not known", key))?;
        if !spec.writable {
            return Err(format!("Setting '{}' is read-only", key));
        }
        if spec.env_wins && first_env(spec).is_some() {
            return Err(format!(
                "Setting '{}' is controlled by the environment",
                key
            ));
        }
        validate_value(spec, value)?;

        {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
        }

        self.values.insert(key.to_string(), value.to_string());
        Ok(self.view(spec))
    }
}
