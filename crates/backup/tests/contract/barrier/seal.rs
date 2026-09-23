//! 导出与封存：未经 WAL checkpoint、未 fsync、未重开核验的副本不能成为封存票据；
//! 传输源不得落在保护根内、不得是硬链接、不得比最短来源保留期活得更久。

use super::super::support::{full_roster, request, transfer_source, verified_checkpoint};
use super::{World, refs, settled};
use studiolo_backup::checkpoint::barrier::{BarrierError, CrossOwnerBarrier};
use studiolo_backup::checkpoint::types::SealError;
use studiolo_core::checkpoint::{CheckpointError, CheckpointOwner, InstantMillis, OwnerWatermark};
use tempfile::TempDir;

#[test]
fn an_unverified_owner_export_cannot_become_a_seal_ticket() {
    let temporary = TempDir::new().expect("tempdir");
    let world = World::new();
    let mut participants = world.participants();
    // WAL checkpoint 仍然 busy：复制在线主文件忽略 WAL 的典型形态。
    participants[2].export.wal.busy = 5;
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    let error = window.seal(
        &request("cp-1", 1_000),
        transfer_source(temporary.path()),
        InstantMillis::from_millis(10),
    );

    assert!(matches!(
        error.err(),
        Some(BarrierError::Owner(CheckpointError::Unverified {
            owner: CheckpointOwner::Auth
        }))
    ));
}

#[test]
fn an_export_that_was_never_reopened_or_synced_is_refused() {
    let world = World::new();
    for break_it in [0_u8, 1, 2, 3] {
        let temporary = TempDir::new().expect("tempdir");
        let mut participants = world.participants();
        match break_it {
            0 => participants[0].export.target_synced = false,
            1 => participants[0].export.parent_directory_synced = false,
            2 => participants[0].export.reopen_verified = false,
            _ => participants[0].export.source_binding_stable = false,
        }
        let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
        let mut window = barrier
            .freeze(&request("cp-1", 1_000), settled(), &world.clock)
            .expect("freeze");
        assert!(
            matches!(
                window.seal(
                    &request("cp-1", 1_000),
                    transfer_source(temporary.path()),
                    InstantMillis::from_millis(10),
                ),
                Err(BarrierError::Owner(CheckpointError::Unverified {
                    owner: CheckpointOwner::Broker
                }))
            ),
            "break_it={break_it}"
        );
    }
}

#[test]
fn a_broker_watermark_that_still_reports_unresolved_blocks_the_seal() {
    let temporary = TempDir::new().expect("tempdir");
    let world = World::new();
    let mut participants = world.participants();
    participants[0].export = verified_checkpoint(CheckpointOwner::Broker, world.vault, world.pair);
    if let OwnerWatermark::Broker { unresolved, .. } = &mut participants[0].export.watermark {
        *unresolved = true;
    }
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    assert!(matches!(
        window.seal(
            &request("cp-1", 1_000),
            transfer_source(temporary.path()),
            InstantMillis::from_millis(10),
        ),
        Err(BarrierError::BrokerUnresolved)
    ));
}

#[test]
fn a_seal_ticket_carries_every_present_owner_and_the_full_roster() {
    let temporary = TempDir::new().expect("tempdir");
    let world = World::new();
    let participants = world.participants();
    let barrier = CrossOwnerBarrier::assemble(refs(&participants)).expect("assemble");
    let mut window = barrier
        .freeze(&request("cp-1", 1_000), settled(), &world.clock)
        .expect("freeze");

    let ticket = window
        .seal(
            &request("cp-1", 1_000),
            transfer_source(temporary.path()),
            InstantMillis::from_millis(10),
        )
        .expect("seal");

    assert_eq!(ticket.owners.len(), 7);
    assert_eq!(ticket.roster, full_roster());
    for owner in CheckpointOwner::FREEZE_ORDER {
        assert!(ticket.owner(owner).is_some(), "{owner:?}");
    }
}

#[test]
fn a_transfer_source_inside_the_vault_or_workbench_data_is_refused() {
    let temporary = TempDir::new().expect("tempdir");
    let vault = temporary.path().join("vault");
    let inside = vault.join("staging");
    let mut source = transfer_source(&inside);
    source.expires_at = InstantMillis::from_millis(1);

    let error = source.validate(
        &vault,
        &temporary.path().join("workbench"),
        InstantMillis::from_millis(10),
    );

    assert_eq!(error, Err(SealError::TransferSourceInsideProtectedRoot));
}

#[test]
fn a_hard_linked_active_file_is_not_an_immutable_view() {
    let temporary = TempDir::new().expect("tempdir");
    let mut source = transfer_source(temporary.path());
    source.materialization = studiolo_backup::checkpoint::types::StagingMaterialization::HardLink;

    assert_eq!(
        source.validate(
            std::path::Path::new("/vault"),
            std::path::Path::new("/workbench"),
            InstantMillis::from_millis(99_999),
        ),
        Err(SealError::HardLinkIsNotImmutable)
    );
}

#[test]
fn a_transfer_source_outliving_the_shortest_retention_would_be_a_hidden_recovery_point() {
    let temporary = TempDir::new().expect("tempdir");
    let source = transfer_source(temporary.path());

    assert_eq!(
        source.validate(
            std::path::Path::new("/vault"),
            std::path::Path::new("/workbench"),
            InstantMillis::from_millis(4_999),
        ),
        Err(SealError::TransferSourceOutlivesRetention)
    );
}
