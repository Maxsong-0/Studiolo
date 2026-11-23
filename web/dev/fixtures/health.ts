/**
 * `/api/v1/health`：整体「降级」，因为同步驱动上次超时，正在退避。
 *
 * 前五节与后端 `crates/observability/src/health/aggregate.rs` 的 code、标题与措辞逐字一致；
 * Bridge、人工配额与备份作业使用真实接口形状（备份作业的正文按 `crates/local-server/src/health/backup.rs`
 * 的写法）；副本与会话仍为标题栏夹具——界面拿不到时必须显示「—」
 * 并写明原因，不能依赖它们一定存在。解码器（`parseHealth`）对未知 code 照单全收。
 */
export function healthBody() {
  return {
    overall: 'degraded',
    evidence_complete: true,
    bridge: {
      last_batch: { sequence: '18', closed_at: '2026-11-13T11:58:00Z' },
      fused: false,
      unknown_jobs: '0',
      quiescence_valid: false,
      conflict_copies: '2',
      mirror_bytes: '1288490188',
      last_exit: 'deadline_exceeded',
      recovery: 'backoff',
      residue_guard: 'clear',
      mode: 'mirror-remote',
      config_matches: true,
      deletes_blocked: '0',
    },
    manual_sync_quota: {
      account_limit_bytes: '10737418240',
      used_bytes_manual: '5368709120',
      recorded_at: '2026-11-13T11:00:00Z',
      as_of: '2026-11-13T12:00:00Z',
      warning_percent: 70,
      max_age_seconds: 86400,
      state: 'normal',
      reason: 'within',
    },
    backup: {
      state: 'normal',
      verdict: 'healthy',
      as_of: '2026-11-13T12:00:00Z',
      last_success: { sequence: '42', at: '2026-11-13T11:20:00Z' },
      outstanding: {
        sequence: '43',
        since: '2026-11-13T11:55:00Z',
        admitted: true,
        stuck: false,
      },
      overdue: false,
      overdue_since: '2026-11-13T11:20:00Z',
      latest_failed: false,
      last_failure: {
        sequence: '41',
        at: '2026-11-13T10:20:00Z',
        result: 'failed',
        reason: 'source_changed_before_seal',
      },
      stuck_after_seconds: '86400',
      overdue_after_seconds: '7200',
    },
    sections: [
      {
        code: 'recovery-point',
        title: '恢复点年龄',
        state: 'normal',
        detail: '最新恢复点 1843 秒前的内容，已在预算内',
      },
      {
        code: 'owner-liveness',
        title: '各 owner 活性',
        state: 'normal',
        detail: '活性回执正常 4 个，超期 0 个，证据不足 0 个',
      },
      {
        code: 'disk-quota',
        title: '磁盘配额',
        state: 'normal',
        detail: '占用在预警水位以内',
      },
      {
        code: 'sync-quota',
        title: '镜像范围配额',
        state: 'normal',
        detail: '占用在预警水位以内',
      },
      {
        code: 'whitelist-diagnosis',
        title: '白名单诊断',
        state: 'normal',
        detail: '检查 24 条登记，未发现冲突',
      },
      {
        code: 'replica-freshness',
        title: '副本',
        state: 'normal',
        detail: '副本 ready，代次 243，距上次确认 111 秒（阈值 300 秒）',
      },
      {
        code: 'sync-bridge',
        title: '双向 Bridge',
        state: 'degraded',
        detail: '上次 headless 运行超过时限被停止；另有：恢复协议进行中',
      },
      {
        code: 'manual-sync-quota',
        title: '账号配额（人工记录）',
        state: 'normal',
        detail: '人工记录低于70%预警；这是抄录时的值，实时账号用量未知',
      },
      {
        code: 'backup-jobs',
        title: '备份作业',
        state: 'normal',
        detail:
          '最近一次成功是 40 分钟前的第 42 号；第 43 号作业在做，已挂 5 分钟',
      },
      {
        code: 'session',
        title: '会话',
        state: 'normal',
        detail: '今日已复核，当前会话 12 小时内有效',
      },
    ],
  };
}
