//! 可信观察 adapter：canonical 完整观察经 `coordinator::sync_bridge` 接缝取得，
//! 镜像观察经 `mirror::observe`；两者合成执行器要的 `ExecutionWitness`。
//!
//! 本文件不持有 Broker 类型、不开文件；证据投影在 `driver::evidence`；镜像观察前先复验静止票据。

use super::driver::evidence::{
    baseline_evidence, inventory_evidence, parse_digest, read_selection, revision_digest,
};
use super::mirror::{self, MirrorObservation, MirrorView};
use super::{PlaneError, SyncPlane, quiescence};
use crate::coordinator::sync_bridge::BridgeSeamError;
use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;
use studiolo_host_config::SyncObservationSelection;
use studiolo_observability::quota::SyncUsage;
use studiolo_sync_bridge::executor::{
    CommonRecheck, ExecutionRecheck, ExecutionWitness, Quiesced, WitnessFault,
};
use studiolo_sync_bridge::{
    ActionKey, ApprovedScope, Baseline, Binding, Digest, FileVersion, Inventory, Presence, Side,
};
use studiolo_sync_transport::owned_run::QuiescentMirror;
use studiolo_vault_broker::CanonicalSyncPresence;

/// 见证的冻结上下文：绑定、已核准范围、原共同基线、打开实例标识与下一对观察序号。
pub struct WitnessContext {
    /// 本次实例的冻结绑定。
    pub binding: Binding,
    /// 已核准范围；观察前可以是空集。
    pub approved: ApprovedScope,
    /// 原完整共同基线；批准联合路径时作为第三份证据。
    pub baseline: Baseline,
    /// 打开实例标识（`BridgeState::observation_epoch`），跨重开不复用。
    pub epoch: Digest,
    /// 下一个 canonical 观察序号。
    pub next_canonical: u64,
    /// 下一个镜像观察序号。
    pub next_mirror: u64,
}

/// 一次执行期间的观察上下文。
pub struct PlaneWitness<'a> {
    plane: &'a SyncPlane,
    view: MirrorView,
    ticket: &'a QuiescentMirror,
    context: WitnessContext,
    utf8: BTreeMap<VaultPath, bool>,
}

impl<'a> PlaneWitness<'a> {
    /// 从冻结上下文构造；观察序号从上下文给的值起。
    #[must_use]
    pub fn new(
        plane: &'a SyncPlane,
        view: MirrorView,
        ticket: &'a QuiescentMirror,
        context: WitnessContext,
    ) -> Self {
        Self {
            plane,
            view,
            ticket,
            context,
            utf8: BTreeMap::new(),
        }
    }

    fn selection(&self) -> Result<SyncObservationSelection, PlaneError> {
        let approval = self.plane.approval();
        approval
            .scope()
            .observation_selection(approval.binding())
            .map_err(|_| PlaneError::Observation)
    }

    /// canonical 侧完整观察：经接缝取 `CanonicalSyncObservation`，逐条换成 `Presence`。
    ///
    /// # Errors
    /// 观察不完整或范围外返回 `Observation`；写者不可用返回 `Unavailable`。
    pub fn observe_canonical(
        &mut self,
        required: &BTreeSet<VaultPath>,
    ) -> Result<Inventory, PlaneError> {
        let selection = read_selection(&self.selection()?);
        let required: Vec<VaultPath> = required.iter().cloned().collect();
        let observation = self
            .plane
            .workbench()
            .bridge_observe(&selection, &required)
            .map_err(seam_error)?;
        let sequence = self.context.next_canonical;
        self.context.next_canonical = sequence.checked_add(1).ok_or(PlaneError::Observation)?;
        let mut entries = BTreeMap::new();
        for (path, presence) in observation.entries() {
            let presence = match presence {
                CanonicalSyncPresence::Missing => {
                    self.utf8.remove(path);
                    Presence::Missing
                }
                CanonicalSyncPresence::Present(file) => {
                    self.utf8.insert(path.clone(), file.utf8_valid);
                    Presence::Present(FileVersion {
                        revision: revision_digest(file.revision),
                        sha256: parse_digest(&file.identity.sha256)
                            .ok_or(PlaneError::Observation)?,
                        bytes: file.identity.bytes,
                    })
                }
            };
            entries.insert(path.clone(), presence);
        }
        Ok(Inventory {
            binding: self.context.binding.clone(),
            side: Side::Canonical,
            sequence,
            complete: true,
            entries,
        })
    }

    /// 镜像侧完整观察（经视图，静止窗口内）：先复验第 7 步的静止票据再遍历。
    ///
    /// # Errors
    /// 票据已作废返回 `Quiescence`；视图身份漂移或遍历失败返回 `Observation`。
    pub fn observe_mirror(
        &mut self,
        required: &BTreeSet<VaultPath>,
    ) -> Result<MirrorObservation, PlaneError> {
        let selection = self.selection()?;
        let sequence = self.context.next_mirror;
        self.context.next_mirror = sequence.checked_add(1).ok_or(PlaneError::Observation)?;
        let required: Vec<VaultPath> = required.iter().cloned().collect();
        quiescence::verify(self.plane, self.ticket)?;
        mirror::observe(
            &self.view,
            &selection,
            &self.context.binding,
            sequence,
            &required,
        )
    }

    /// 两侧完整观察并收敛到同一路径集：每侧都把另一侧发现的路径列为 required，
    /// 缺失因此都是显式 `Missing`；两侧路径集仍不一致（观察期间有人写入）即失败。
    ///
    /// # Errors
    /// 任一侧观察失败或路径集不收敛返回 `Observation`。
    pub fn observe_pair(
        &mut self,
        required: &BTreeSet<VaultPath>,
    ) -> Result<(Inventory, Inventory, SyncUsage), PlaneError> {
        let mut canonical = self.observe_canonical(required)?;
        let mut union = required.clone();
        union.extend(canonical.entries.keys().cloned());
        let mirror = self.observe_mirror(&union)?;
        if mirror
            .inventory
            .entries
            .keys()
            .any(|path| !canonical.entries.contains_key(path))
        {
            union.extend(mirror.inventory.entries.keys().cloned());
            canonical = self.observe_canonical(&union)?;
        }
        if !canonical.entries.keys().eq(mirror.inventory.entries.keys()) {
            return Err(PlaneError::Observation);
        }
        Ok((canonical, mirror.inventory, mirror.usage))
    }

    /// 经 `SyncScope::approve_union` 核准基线与两侧观察的完整联合路径，并记为当前范围。
    ///
    /// # Errors
    /// 任一路径未批准、类型不可批准或超预算返回 `Observation`。
    pub fn approve(
        &mut self,
        canonical: &Inventory,
        mirror: &Inventory,
    ) -> Result<ApprovedScope, PlaneError> {
        let approval = self.plane.approval();
        let union = approval
            .scope()
            .approve_union(
                approval.binding(),
                &baseline_evidence(&self.context.baseline)?,
                &inventory_evidence(canonical, &self.utf8)?,
                &inventory_evidence(mirror, &self.utf8)?,
            )
            .map_err(|_| PlaneError::Observation)?;
        self.context.approved = ApprovedScope {
            binding: self.context.binding.clone(),
            paths: union.paths().keys().cloned().collect(),
        };
        Ok(self.context.approved.clone())
    }

    /// 已核准范围。
    #[must_use]
    pub fn approved(&self) -> &ApprovedScope {
        &self.context.approved
    }

    /// 下一对观察序号 `(canonical, mirror)`。
    #[must_use]
    pub fn next_sequences(&self) -> (u64, u64) {
        (self.context.next_canonical, self.context.next_mirror)
    }
}

/// 接缝失败到平面失败的映射；`NotAttempted`／`Rejected`／`Unknown` 在观察语境下都只是「写者不可用」。
pub(super) fn seam_error(error: BridgeSeamError) -> PlaneError {
    match error {
        BridgeSeamError::Incomplete => PlaneError::Observation,
        BridgeSeamError::Audit => PlaneError::Audit,
        BridgeSeamError::NotAttempted
        | BridgeSeamError::Unavailable
        | BridgeSeamError::Rejected
        | BridgeSeamError::Unknown => PlaneError::Unavailable,
    }
}

fn witness_fault(error: PlaneError) -> WitnessFault {
    match error {
        PlaneError::Observation => WitnessFault::Incomplete,
        _ => WitnessFault::Unavailable,
    }
}

impl ExecutionWitness for PlaneWitness<'_> {
    fn recheck(
        &mut self,
        key: &ActionKey,
        quiesced: &Quiesced,
    ) -> Result<ExecutionRecheck, WitnessFault> {
        let required = BTreeSet::from([key.path.clone()]);
        let canonical = self.observe_canonical(&required).map_err(witness_fault)?;
        let mirror = self.observe_mirror(&required).map_err(witness_fault)?;
        let on_canonical = canonical
            .entries
            .get(&key.path)
            .cloned()
            .ok_or(WitnessFault::Incomplete)?;
        let on_mirror = mirror
            .inventory
            .entries
            .get(&key.path)
            .cloned()
            .ok_or(WitnessFault::Incomplete)?;
        let (source, target) = match key.target {
            Side::Canonical => (on_mirror, on_canonical),
            Side::Mirror => (on_canonical, on_mirror),
        };
        Ok(ExecutionRecheck {
            binding: self.context.binding.clone(),
            source,
            target,
            canonical_observation: canonical.sequence,
            mirror_observation: mirror.inventory.sequence,
            witness_epoch: self.context.epoch,
            expires_at: quiesced.deadline(),
        })
    }

    fn common(&mut self, quiesced: &Quiesced) -> Result<Option<CommonRecheck>, WitnessFault> {
        let required = self.context.approved.paths.clone();
        let (canonical, mirror, _) = match self.observe_pair(&required) {
            Ok(observed) => observed,
            Err(PlaneError::Observation) => return Ok(None),
            Err(error) => return Err(witness_fault(error)),
        };
        if !canonical.complete || !mirror.complete {
            return Ok(None);
        }
        let approved = match self.approve(&canonical, &mirror) {
            Ok(approved) => approved,
            Err(PlaneError::Observation) => return Ok(None),
            Err(error) => return Err(witness_fault(error)),
        };
        Ok(Some(CommonRecheck {
            canonical,
            mirror,
            approved,
            witness_epoch: self.context.epoch,
            expires_at: quiesced.deadline(),
        }))
    }
}
