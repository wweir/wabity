use std::collections::{HashMap, HashSet};

use crate::infrastructure::config::SavedAcpSession;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotWriteMode {
    InsertOrUpdate,
    UpdateOnly,
}

pub(super) fn merge_restored_session_snapshots(
    current_snapshots: Vec<SavedAcpSession>,
    initial_snapshot_ids: &HashSet<String>,
    retained_snapshots: Vec<SavedAcpSession>,
) -> Vec<SavedAcpSession> {
    let retained_by_id = retained_snapshots
        .into_iter()
        .map(|snapshot| (snapshot.session_id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let mut merged = current_snapshots
        .into_iter()
        .filter(|snapshot| {
            !initial_snapshot_ids.contains(&snapshot.session_id)
                || retained_by_id.contains_key(&snapshot.session_id)
        })
        .collect::<Vec<_>>();

    for snapshot in retained_by_id.into_values() {
        match merged
            .iter_mut()
            .find(|item| item.session_id == snapshot.session_id)
        {
            Some(existing) if existing.last_updated_at_ms >= snapshot.last_updated_at_ms => {}
            Some(existing) => *existing = snapshot,
            None => merged.push(snapshot),
        }
    }

    sort_and_truncate_snapshots(&mut merged);
    merged
}

#[cfg(test)]
pub(super) fn apply_session_snapshot(
    saved_sessions: &mut Vec<SavedAcpSession>,
    snapshot: SavedAcpSession,
    write_mode: SnapshotWriteMode,
) -> bool {
    let existing_index = saved_sessions
        .iter()
        .position(|item| item.session_id == snapshot.session_id);

    match (existing_index, write_mode) {
        (Some(index), _) => {
            if !should_replace_existing_snapshot(&saved_sessions[index], &snapshot) {
                return false;
            }
            saved_sessions[index] = snapshot;
            true
        }
        (None, SnapshotWriteMode::InsertOrUpdate) => {
            saved_sessions.push(snapshot);
            true
        }
        (None, SnapshotWriteMode::UpdateOnly) => false,
    }
}

#[cfg(test)]
fn should_replace_existing_snapshot(
    existing: &SavedAcpSession,
    incoming: &SavedAcpSession,
) -> bool {
    existing.last_updated_at_ms < incoming.last_updated_at_ms
}

fn sort_and_truncate_snapshots(snapshots: &mut Vec<SavedAcpSession>) {
    snapshots.sort_by(|left, right| {
        right
            .last_updated_at_ms
            .cmp(&left.last_updated_at_ms)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    snapshots.truncate(16);
}
