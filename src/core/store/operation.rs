use std::fs;
use std::io;

use super::fs::{DaggerPaths, ensure_store_dir, write_atomic};
use super::types::PendingOperationState;

pub fn load_operation(paths: &DaggerPaths) -> io::Result<Option<PendingOperationState>> {
    if !paths.operation_file.exists() {
        return Ok(None);
    }

    let bytes = fs::read(&paths.operation_file)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    let migrated = super::migrate::migrate_operation(value)?;
    let operation: PendingOperationState = serde_json::from_value(migrated)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    Ok(Some(operation))
}

pub fn save_operation(paths: &DaggerPaths, operation: &PendingOperationState) -> io::Result<()> {
    ensure_store_dir(paths)?;

    let bytes = serde_json::to_vec_pretty(operation)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    write_atomic(&paths.operation_file, &bytes)
}

pub fn clear_operation(paths: &DaggerPaths) -> io::Result<()> {
    if !paths.operation_file.exists() {
        return Ok(());
    }

    fs::remove_file(&paths.operation_file)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use uuid::Uuid;

    use super::{clear_operation, load_operation, save_operation};
    use crate::core::restack::{RestackAction, RestackBaseTarget};
    use crate::core::store::dagger_paths;
    use crate::core::store::{
        ParentRef, PendingCommitOperation, PendingOperationKind, PendingOperationState,
        PendingReparentOperation, PendingSyncOperation, PendingSyncPhase,
    };

    #[test]
    fn saves_and_loads_pending_operation() {
        let git_dir = std::env::temp_dir().join(format!("dgr-operation-{}", Uuid::new_v4()));
        fs::create_dir_all(&git_dir).unwrap();

        let paths = dagger_paths(&git_dir);
        let operation = PendingOperationState::start(
            PendingOperationKind::Commit(PendingCommitOperation {
                current_branch: "feat/auth".into(),
                summary_line: Some("1 file changed".into()),
                recent_commits: Vec::new(),
            }),
            vec![RestackAction {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth-ui".into(),
                old_upstream_branch_name: "feat/auth".into(),
                old_upstream_oid: "old".into(),
                new_base: RestackBaseTarget::local("feat/auth"),
                new_parent: Some(ParentRef::Trunk),
            }],
        )
        .unwrap();

        save_operation(&paths, &operation).unwrap();

        assert_eq!(load_operation(&paths).unwrap(), Some(operation));

        fs::remove_dir_all(git_dir).unwrap();
    }

    #[test]
    fn clears_pending_operation_file() {
        let git_dir = std::env::temp_dir().join(format!("dgr-operation-{}", Uuid::new_v4()));
        fs::create_dir_all(&git_dir).unwrap();

        let paths = dagger_paths(&git_dir);
        let operation = PendingOperationState::start(
            PendingOperationKind::Commit(PendingCommitOperation {
                current_branch: "feat/auth".into(),
                summary_line: Some("1 file changed".into()),
                recent_commits: Vec::new(),
            }),
            vec![RestackAction {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth-ui".into(),
                old_upstream_branch_name: "feat/auth".into(),
                old_upstream_oid: "old".into(),
                new_base: RestackBaseTarget::local("feat/auth"),
                new_parent: Some(ParentRef::Trunk),
            }],
        )
        .unwrap();

        save_operation(&paths, &operation).unwrap();
        clear_operation(&paths).unwrap();

        assert_eq!(load_operation(&paths).unwrap(), None);

        fs::remove_dir_all(git_dir).unwrap();
    }

    #[test]
    fn saves_and_loads_pending_sync_operation() {
        let git_dir = std::env::temp_dir().join(format!("dgr-operation-{}", Uuid::new_v4()));
        fs::create_dir_all(&git_dir).unwrap();

        let paths = dagger_paths(&git_dir);
        let operation = PendingOperationState::start(
            PendingOperationKind::Sync(PendingSyncOperation {
                original_branch: "feat/auth".into(),
                remote_sync_enabled: true,
                deleted_branches: vec!["feat/missing".into()],
                restacked_branches: Vec::new(),
                phase: PendingSyncPhase::RestackOutdatedLocalStacks,
                step_branch_name: "feat/auth".into(),
            }),
            vec![RestackAction {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth".into(),
                old_upstream_branch_name: "main".into(),
                old_upstream_oid: "old".into(),
                new_base: RestackBaseTarget::local("main"),
                new_parent: None,
            }],
        )
        .unwrap();

        save_operation(&paths, &operation).unwrap();

        assert_eq!(load_operation(&paths).unwrap(), Some(operation));

        fs::remove_dir_all(git_dir).unwrap();
    }

    #[test]
    fn saves_and_loads_pending_reparent_operation() {
        let git_dir = std::env::temp_dir().join(format!("dgr-operation-{}", Uuid::new_v4()));
        fs::create_dir_all(&git_dir).unwrap();

        let paths = dagger_paths(&git_dir);
        let operation = PendingOperationState::start(
            PendingOperationKind::Reparent(PendingReparentOperation {
                original_branch: "feat/current".into(),
                branch_name: "feat/auth".into(),
                parent_branch_name: "feat/platform".into(),
            }),
            vec![RestackAction {
                node_id: Uuid::new_v4(),
                branch_name: "feat/auth".into(),
                old_upstream_branch_name: "main".into(),
                old_upstream_oid: "old".into(),
                new_base: RestackBaseTarget::local("feat/platform"),
                new_parent: Some(ParentRef::Trunk),
            }],
        )
        .unwrap();

        save_operation(&paths, &operation).unwrap();

        assert_eq!(load_operation(&paths).unwrap(), Some(operation));

        fs::remove_dir_all(git_dir).unwrap();
    }
}
