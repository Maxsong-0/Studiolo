/**
 * `/api/v1/health`：整体「降级」，因为副本同步每轮 85 秒、逼近 120 秒时限。
 *
 * 前五节与后端 `crates/observability/src/health/aggregate.rs` 的 code、标题与措辞逐字一致；
 * 后三节（副本、同步、会话）是画布标题栏要的读数，当前后端还不发——界面拿不到时必须显示「—」
 * 并写明原因，不能依赖它们一定存在。解码器（`parseHealth`）对未知 code 照单全收。
 */
export function healthBody() {
  return {
    overall: 'degraded',
    evidence_complete: true,
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
        title: 'Sync 配额',
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
        code: 'sync-round',
        title: '副本同步',
        state: 'degraded',
        detail: '每轮 85 秒，时限 120 秒；余量不足三成，全量重推不可持续',
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
