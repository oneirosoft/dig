use std::io;

use uuid::Uuid;

use super::{
    BranchAdoptedEvent, BranchArchiveReason, BranchArchivedEvent, BranchCreatedEvent,
    BranchDivergenceState, BranchNode, BranchPullRequestTrackedEvent,
    BranchPullRequestTrackedSource, BranchReparentedEvent, DigEvent, ParentRef, TrackedPullRequest,
    now_unix_timestamp_secs, save_state,
};
use crate::core::store::append_event;
use crate::core::store::session::StoreSession;

pub fn record_branch_created(session: &mut StoreSession, node: BranchNode) -> io::Result<()> {
    session.state.insert_branch(node.clone())?;
    save_state(&session.paths, &session.state)?;
    append_event(
        &session.paths,
        &DigEvent::BranchCreated(BranchCreatedEvent {
            occurred_at_unix_secs: now_unix_timestamp_secs(),
            node,
        }),
    )
}

pub fn record_branch_adopted(session: &mut StoreSession, node: BranchNode) -> io::Result<()> {
    session.state.insert_branch(node.clone())?;
    save_state(&session.paths, &session.state)?;
    append_event(
        &session.paths,
        &DigEvent::BranchAdopted(BranchAdoptedEvent {
            occurred_at_unix_secs: now_unix_timestamp_secs(),
            node,
        }),
    )
}

pub fn record_branch_reparented(
    session: &mut StoreSession,
    branch_id: Uuid,
    branch_name: String,
    old_parent: ParentRef,
    new_parent: ParentRef,
    old_base_ref: String,
    new_base_ref: String,
) -> io::Result<()> {
    save_state(&session.paths, &session.state)?;
    append_event(
        &session.paths,
        &DigEvent::BranchReparented(BranchReparentedEvent {
            occurred_at_unix_secs: now_unix_timestamp_secs(),
            branch_id,
            branch_name,
            old_parent,
            new_parent,
            old_base_ref,
            new_base_ref,
        }),
    )
}

pub fn record_branch_archived(
    session: &mut StoreSession,
    branch_id: Uuid,
    branch_name: String,
    reason: BranchArchiveReason,
) -> io::Result<()> {
    session.state.archive_branch(branch_id)?;
    save_state(&session.paths, &session.state)?;
    append_event(
        &session.paths,
        &DigEvent::BranchArchived(BranchArchivedEvent {
            occurred_at_unix_secs: now_unix_timestamp_secs(),
            branch_id,
            branch_name,
            reason,
        }),
    )
}

pub fn record_branch_pull_request_tracked(
    session: &mut StoreSession,
    branch_id: Uuid,
    branch_name: String,
    pull_request: TrackedPullRequest,
    source: BranchPullRequestTrackedSource,
) -> io::Result<()> {
    session
        .state
        .track_pull_request(branch_id, pull_request.clone())?;
    save_state(&session.paths, &session.state)?;
    append_event(
        &session.paths,
        &DigEvent::BranchPullRequestTracked(BranchPullRequestTrackedEvent {
            occurred_at_unix_secs: now_unix_timestamp_secs(),
            branch_id,
            branch_name,
            pull_request,
            source,
        }),
    )
}

pub fn record_branch_divergence_state(
    session: &mut StoreSession,
    branch_id: Uuid,
    divergence_state: BranchDivergenceState,
) -> io::Result<bool> {
    if !session
        .state
        .set_branch_divergence_state(branch_id, divergence_state)?
    {
        return Ok(false);
    }

    save_state(&session.paths, &session.state)?;
    Ok(true)
}
