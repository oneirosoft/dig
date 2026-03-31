use std::io;

use serde_json::Value;

use super::types::{DAGGER_CONFIG_VERSION, DAGGER_OPERATION_VERSION, DAGGER_STATE_VERSION};

/// Migrate a state JSON value from its current version to DAGGER_STATE_VERSION.
/// Returns the migrated value, or the original if already current.
pub fn migrate_state(mut value: Value) -> io::Result<Value> {
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "state file missing 'version' field",
            )
        })? as u32;

    if version > DAGGER_STATE_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "state version {} is newer than supported version {}; upgrade dgr",
                version, DAGGER_STATE_VERSION
            ),
        ));
    }

    if version == DAGGER_STATE_VERSION {
        return Ok(value);
    }

    // Apply migrations sequentially: 0→1, 1→2, etc.
    // Currently no migrations needed since we're at version 1.
    // Future migrations would be added here:
    // if version < 2 { value = migrate_state_v1_to_v2(value)?; }

    // Update the version field after all migrations
    value["version"] = serde_json::json!(DAGGER_STATE_VERSION);

    Ok(value)
}

/// Migrate a config JSON value from its current version to DAGGER_CONFIG_VERSION.
/// Returns the migrated value, or the original if already current.
pub fn migrate_config(mut value: Value) -> io::Result<Value> {
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "config file missing 'version' field",
            )
        })? as u32;

    if version > DAGGER_CONFIG_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "config version {} is newer than supported version {}; upgrade dgr",
                version, DAGGER_CONFIG_VERSION
            ),
        ));
    }

    if version == DAGGER_CONFIG_VERSION {
        return Ok(value);
    }

    value["version"] = serde_json::json!(DAGGER_CONFIG_VERSION);

    Ok(value)
}

/// Migrate an operation JSON value from its current version to DAGGER_OPERATION_VERSION.
/// Returns the migrated value, or the original if already current.
pub fn migrate_operation(mut value: Value) -> io::Result<Value> {
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "operation file missing 'version' field",
            )
        })? as u32;

    if version > DAGGER_OPERATION_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "operation version {} is newer than supported version {}; upgrade dgr",
                version, DAGGER_OPERATION_VERSION
            ),
        ));
    }

    if version == DAGGER_OPERATION_VERSION {
        return Ok(value);
    }

    value["version"] = serde_json::json!(DAGGER_OPERATION_VERSION);

    Ok(value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn state_current_version_passes_through_unchanged() {
        let input = json!({
            "version": DAGGER_STATE_VERSION,
            "nodes": [
                {
                    "id": "00000000-0000-0000-0000-000000000000",
                    "branch_name": "feat/api",
                    "parent": {"kind": "trunk"},
                    "base_ref": "main",
                    "fork_point_oid": "abc123",
                    "head_oid_at_creation": "abc123",
                    "created_at_unix_secs": 1,
                    "archived": false
                }
            ]
        });

        let result = migrate_state(input.clone()).unwrap();

        assert_eq!(result, input);
    }

    #[test]
    fn state_future_version_returns_upgrade_error() {
        let input = json!({
            "version": DAGGER_STATE_VERSION + 1,
            "nodes": []
        });

        let err = migrate_state(input).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("upgrade dgr"));
    }

    #[test]
    fn state_missing_version_returns_error() {
        let input = json!({
            "nodes": []
        });

        let err = migrate_state(input).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("missing 'version' field"));
    }

    #[test]
    fn state_migration_preserves_all_fields() {
        // Simulate a value at version 0 (older than current) that needs migration.
        // Since DAGGER_STATE_VERSION is 1 and we have no real v0→v1 migration,
        // this tests the framework: version gets bumped, other fields are preserved.
        let input = json!({
            "version": 0,
            "nodes": [
                {
                    "id": "00000000-0000-0000-0000-000000000001",
                    "branch_name": "feat/login",
                    "parent": {"kind": "trunk"},
                    "base_ref": "main",
                    "fork_point_oid": "def456",
                    "head_oid_at_creation": "def456",
                    "created_at_unix_secs": 100,
                    "archived": false
                }
            ]
        });

        let result = migrate_state(input).unwrap();

        assert_eq!(result["version"], json!(DAGGER_STATE_VERSION));
        assert_eq!(result["nodes"][0]["branch_name"], json!("feat/login"));
        assert_eq!(result["nodes"][0]["fork_point_oid"], json!("def456"));
    }

    #[test]
    fn config_current_version_passes_through_unchanged() {
        let input = json!({
            "version": DAGGER_CONFIG_VERSION,
            "trunk_branch": "main"
        });

        let result = migrate_config(input.clone()).unwrap();

        assert_eq!(result, input);
    }

    #[test]
    fn config_future_version_returns_upgrade_error() {
        let input = json!({
            "version": DAGGER_CONFIG_VERSION + 1,
            "trunk_branch": "main"
        });

        let err = migrate_config(input).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("upgrade dgr"));
    }

    #[test]
    fn config_missing_version_returns_error() {
        let input = json!({
            "trunk_branch": "main"
        });

        let err = migrate_config(input).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("missing 'version' field"));
    }

    #[test]
    fn operation_current_version_passes_through_unchanged() {
        let input = json!({
            "version": DAGGER_OPERATION_VERSION,
            "origin": {"type": "commit", "current_branch": "feat/api", "summary_line": null, "recent_commits": []},
            "restack": {
                "active_action": {
                    "node_id": "00000000-0000-0000-0000-000000000000",
                    "branch_name": "feat/api",
                    "old_upstream_branch_name": "main",
                    "old_upstream_oid": "abc",
                    "new_base": {"branch_name": "main", "source": "local"},
                    "new_parent": null
                },
                "remaining_actions": [],
                "completed_branches": []
            }
        });

        let result = migrate_operation(input.clone()).unwrap();

        assert_eq!(result, input);
    }

    #[test]
    fn operation_future_version_returns_upgrade_error() {
        let input = json!({
            "version": DAGGER_OPERATION_VERSION + 1,
            "origin": {"type": "commit", "current_branch": "feat/api", "summary_line": null, "recent_commits": []},
            "restack": {"active_action": {}, "remaining_actions": [], "completed_branches": []}
        });

        let err = migrate_operation(input).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("upgrade dgr"));
    }

    #[test]
    fn operation_missing_version_returns_error() {
        let input = json!({
            "origin": {"type": "commit"}
        });

        let err = migrate_operation(input).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("missing 'version' field"));
    }
}
