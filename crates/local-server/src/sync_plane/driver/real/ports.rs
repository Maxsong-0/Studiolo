//! 一轮端口的机械步骤；状态、静止票据与私有能力由父模块持有。

use super::*;

impl RoundPorts for RealPorts<'_> {
    fn state(&mut self) -> Result<PlaneState, PlaneError> {
        Ok(self.store.state().clone())
    }

    fn inspect(&mut self, expected: SyncMode) -> Result<Inspection, PlaneError> {
        self.mode = expected;
        let run = self.run_headless(RunOperation::InspectConfig)?;
        if !run.clean {
            return Err(run.failure());
        }
        let output = self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .config_output(run.run_id)
            .map_err(|_| PlaneError::Transport)?;
        let plane = self.plane;
        let inspection = headless::classify(
            expected,
            plane.approval().excluded_folders(),
            &self.store.state().verified_excluded,
            &mut |excluded, mode| {
                headless::verify_config(plane, output.as_bytes(), mode, excluded).is_ok()
            },
        )?;
        if inspection.mode_matches {
            plane.health.record_mode(expected);
        }
        Ok(inspection)
    }

    fn residue_blocked(
        &mut self,
        previous: &[String],
        current: &[String],
    ) -> Result<bool, PlaneError> {
        // 守卫要读镜像视图：先取 InspectConfig 刚闭合留下的静止票据，读完复验，作废即 `Quiescence`。
        let ticket = self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .quiescent()
            .map_err(|_| PlaneError::Quiescence)?;
        let residue = recovery::residue_guard(previous, current, ticket.mirror_root())?;
        quiescence::verify(self.plane, &ticket)?;
        Ok(residue.is_some())
    }

    fn set_mode(&mut self, mode: SyncMode) -> Result<HeadlessRun, PlaneError> {
        self.run_headless(RunOperation::SetMode(mode))
    }

    fn next_sequence(&mut self) -> Result<u64, PlaneError> {
        Ok(self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .inspect()
            .map_err(headless::transport_error)?
            .next_sequence)
    }

    fn run_once(&mut self) -> Result<HeadlessRun, PlaneError> {
        self.run_headless(RunOperation::RunOnce)
    }

    fn save_state(&mut self, state: PlaneState) -> Result<(), PlaneError> {
        self.store.save(state)
    }

    fn clock(&mut self) -> Option<i64> {
        Some(now_unix_seconds())
    }

    fn quiescent(&mut self) -> Result<(), PlaneError> {
        let ticket = self
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .quiescent()
            .map_err(|_| PlaneError::Quiescence)?;
        self.ticket = Some(ticket);
        Ok(())
    }

    fn unresolved(&mut self) -> Result<Option<u64>, PlaneError> {
        // 第 10 步续做：上一批仍有 Issued 未知终态的动作，先按原键向后端核对权威终态并写回执，
        // 核不出的才保持未知（设计文档第 2 节第 10 步「下一轮原键核对」，评审 P1-1）。
        if self.pending_unknown()?.1 > 0 {
            reconcile::reconcile_pending(&self.context()?)?;
        }
        let (sequence, unknown) = self.pending_unknown()?;
        Ok((unknown > 0).then_some(sequence).flatten())
    }

    fn observe(&mut self, mode: SyncMode) -> Result<(Input, Inventory, Inventory), PlaneError> {
        self.mode = mode;
        let observed = observe::observe_both(&self.context()?)?;
        let result = (
            observed.input.clone(),
            observed.canonical.clone(),
            observed.mirror.clone(),
        );
        self.observed = Some(observed);
        Ok(result)
    }

    fn preresolve(&mut self, decision: &Decision) -> Result<bool, PlaneError> {
        // 戳先落盘、后保全与 CAS：中途失败的下一轮沿用同一戳、同一副本路径与幂等键（评审 P1-2）。
        let link_id = self.context()?.binding.link_id;
        preserve::stamp(&mut self.store, &link_id, decision, self.now_unix)?;
        self.plane
            .health
            .record_conflicts(self.store.state().conflict_stamps.len() as u64);
        let stamps = &self.store.state().conflict_stamps;
        resolve::preresolve_conflicts(&self.context()?, decision, stamps)
    }

    fn preserve_conflicts(&mut self, decision: &Decision) -> Result<u64, PlaneError> {
        // 戳先落盘、后写副本：写到一半失败或进程中断，下一轮沿用同一戳、同一路径与幂等键，不另生一份。
        // 冲突一经配戳落盘即计入健康节（副本由本轮或后续轮按同一戳幂等写出）。
        let link_id = self.context()?.binding.link_id;
        let added = preserve::stamp(&mut self.store, &link_id, decision, self.now_unix)?;
        self.plane
            .health
            .record_conflicts(self.store.state().conflict_stamps.len() as u64);
        let device = self.plane.approval().device_name();
        let stamps = &self.store.state().conflict_stamps;
        preserve::preserve_conflicts(&self.context()?, decision, stamps, device)?;
        Ok(added)
    }

    fn retain_conflicts(&mut self, decision: &Decision) -> Result<(), PlaneError> {
        let link_id = self.context()?.binding.link_id;
        preserve::prune(&mut self.store, &link_id, decision)?;
        self.plane
            .health
            .record_conflicts(self.store.state().conflict_stamps.len() as u64);
        Ok(())
    }

    fn execute(&mut self, input: Input) -> Result<RoundOutcome, PlaneError> {
        let observed = self.observed.take().ok_or(PlaneError::Observation)?;
        if observed.input != input {
            return Err(PlaneError::Observation);
        }
        let outcome = execute::reserve_and_execute(&self.context()?, &observed, self.stopping);
        // 执行段回填（W5 `record_execution`）：序号、终态未知数、熔断是否生效；冲突副本数由健康侧合计。
        if let Ok(
            RoundOutcome::Settled { sequence, .. }
            | RoundOutcome::Unknown { sequence }
            | RoundOutcome::Blocked { sequence },
        ) = outcome
        {
            let (_, unknown) = self.pending_unknown()?;
            let fused = self
                .plane
                .bridge()
                .lock()
                .map_err(|_| PlaneError::Unavailable)?
                .view()
                .map(|view| view.fused)
                .unwrap_or(true);
            let health = &self.plane.health;
            health.record_execution(sequence, unknown, fused);
        }
        outcome
    }

    fn stopping(&mut self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
}
