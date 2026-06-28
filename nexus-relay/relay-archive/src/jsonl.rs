use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use relay_db::ledger::LedgerEntryRow;
use uuid::Uuid;

pub struct ArchiveEntry {
    pub entry: LedgerEntryRow,
    pub namespace_name: String,
    pub host: Option<String>,
    pub agent_name: Option<String>,
    pub is_operator: bool,
}

pub fn entry_to_jsonl_line(entry: &LedgerEntryRow) -> String {
    let obj = serde_json::json!({
        "id": entry.id.to_string(),
        "ledger_id": entry.ledger_id.to_string(),
        "sequence": entry.sequence,
        "received_at": entry.received_at.to_rfc3339(),
        "sender_id": entry.sender_id.to_string(),
        "msg_type": entry.msg_type,
        "correlation_id": entry.correlation_id.map(|u: Uuid| u.to_string()),
        "sent_at": entry.sent_at.map(|t: DateTime<Utc>| t.to_rfc3339()),
        "payload": entry.payload,
        "attachments": entry.attachments,
    });
    format!("{}\n", serde_json::to_string(&obj).unwrap())
}

pub fn archive_path(entry: &ArchiveEntry) -> String {
    let date = entry.entry.received_at.format("%Y-%m-%d").to_string();
    if entry.is_operator {
        format!("{}/_operator/{}.jsonl", entry.namespace_name, date)
    } else {
        let host = entry.host.as_deref().unwrap_or("unknown");
        let agent = entry.agent_name.as_deref().unwrap_or("unknown");
        format!("{}/{}/{}/{}.jsonl", entry.namespace_name, host, agent, date)
    }
}

pub fn group_entries_by_file(entries: Vec<ArchiveEntry>) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ae in entries {
        let path = archive_path(&ae);
        let line = entry_to_jsonl_line(&ae.entry);
        map.entry(path).or_default().push(line);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn mock_entry() -> LedgerEntryRow {
        LedgerEntryRow {
            id: Uuid::new_v4(),
            ledger_id: Uuid::new_v4(),
            sequence: 1,
            received_at: Utc::now(),
            sender_id: Uuid::new_v4(),
            msg_type: "test.message".to_string(),
            correlation_id: None,
            sent_at: None,
            payload: serde_json::json!({"title": "Hello"}),
            attachments: None,
        }
    }

    #[test]
    fn test_entry_to_jsonl_line() {
        let entry = mock_entry();
        let line = entry_to_jsonl_line(&entry);
        assert!(line.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["id"], entry.id.to_string());
        assert_eq!(parsed["ledger_id"], entry.ledger_id.to_string());
        assert_eq!(parsed["sequence"], 1i64);
        assert_eq!(parsed["msg_type"], "test.message");
        assert!(parsed["payload"].is_object());
    }

    #[test]
    fn test_archive_path_operator() {
        let entry = mock_entry();
        let date = entry.received_at.format("%Y-%m-%d").to_string();
        let ae = ArchiveEntry {
            entry,
            namespace_name: "acme".to_string(),
            host: None,
            agent_name: None,
            is_operator: true,
        };
        assert_eq!(archive_path(&ae), format!("acme/_operator/{}.jsonl", date));
    }

    #[test]
    fn test_archive_path_agent() {
        let entry = mock_entry();
        let date = entry.received_at.format("%Y-%m-%d").to_string();
        let ae = ArchiveEntry {
            entry,
            namespace_name: "acme".to_string(),
            host: Some("host1".to_string()),
            agent_name: Some("agent-alpha".to_string()),
            is_operator: false,
        };
        assert_eq!(
            archive_path(&ae),
            format!("acme/host1/agent-alpha/{}.jsonl", date)
        );
    }
}
