# 同步状态回执契约（`sync-status.json`）

T-224，2026-11-22（BUG-046 防回归）。人侧 Bridge 驱动平面每轮写一份小回执，F4 的 169 探针读它，
判「同步两边没对上满 6 小时」（A07）和「删除被挡、同步整批暂停」（A08）。人侧只写事实，两条线都在探针里。
健康页（`GET /api/v1/health`）和第六节判定不受影响，见 [health-sync-bridge](health-sync-bridge.md)。

实现位置：

- 编解码与重启起点：`crates/local-server/src/sync_plane/status.rs`（纯逻辑，没有 IO）。
- 事实来源：`crates/local-server/src/sync_plane/health.rs`（`PlaneHealth::record_round`、`status_receipt`、`resume_status`）。
- 读写：`crates/local-server/src/sync_plane/recovery.rs`（`PlaneStateStore::read_status`／`write_status`，复用外 stage 的描述符助手）。
- 何时写：`crates/local-server/src/sync_plane.rs`（`SyncPlane::open`、`drive_until`）。
- 读法：`deploy/alerts/probe.py`（`read_sync_status`、`sync_flags`），代号与节奏 `deploy/alerts/watch_state.py`。

## 1. 文件

| 项 | 约定 |
|---|---|
| 位置 | `<plane_root>/sync-status.json`；169 上是 `/var/lib/studiolo-owner/bridge/plane/sync-status.json` |
| 属主与权限 | 人侧服务用户本人、0600、单链接；父目录本人 0700。F4 169 探针以同一用户运行 |
| 大小 | 不超过 4096 字节 |
| 写法 | 唯一暂存件 `sync-status.json.new`：清陈旧暂存件 → 独占建 → 写满 → fsync → renameat → fsync 目录。不会留下半个回执 |
| 和别的文件 | 不碰 `plane-state.json`（那份拒绝未知字段，「最后对上」放进去新发行就退不回上一版）；备份不导出它，恢复之后按「没有回执」起算 |

## 2. 字段（闭合四个，一个都不能少）

紧凑 JSON 对象，时刻都是 Unix 秒整数，可空的写 `null`：

| 字段 | 形状 | 含义 |
|---|---|---|
| `schema` | `1` | 版本 |
| `written_at` | 非负整数 | 这份回执写出的时刻 |
| `last_aligned_at` | `null` 或 `0..=written_at` 的整数 | 两边最后一次对上的那一轮开始的时刻；`null` 即说不清 |
| `deletes_blocked` | `null` 或非负整数（u64） | 最近一次走到删除守卫那一轮被挡的主库删除数（BUG-046）；`null` 即还没有一轮证明过 |

例：`{"schema":1,"written_at":1800000000,"last_aligned_at":1799999700,"deletes_blocked":0}`

缺字段、多字段、重复键、类型不对（含 `true` 冒充整数、小数、负数）、版本不是 1、`last_aligned_at` 晚于 `written_at`、
超过 4096 字节：两边都判读不出来。

## 3. 「对上」和写出时机

- **对上**＝一轮以 `Idle`（规划结果全是共同观察）或 `Settled`（一批闭合）结束（`status::aligned`）。
  卡在终态未知、冻结、删除被挡、退避、强制恢复、任何错误都不算。时刻取 `drive_until` 给这一轮的时刻（轮次开始）。
  写出时对上时刻晚于写出时刻（时钟回拨）就写 `null`，不写出一份自己都读不回的回执。
- **开平面时**：先读上一份回执接上起点（下表），立刻写一份——第一轮要等一个轮间隔（300 秒），探针不能这段时间读不到。
- **每轮收口后**：`record_round` 之后短暂重开平面租约写一份。租约正被备份冻结占着、或 `plane-state.json` 打不开，
  就跳过这一轮，回执停在上一份。写失败只留一行日志 `studiolo_bridge component=sync_status outcome=unwritten`，不影响本轮结论。
- `deletes_blocked` 取本进程删除守卫的结论；守卫还没跑过（刚重启），沿用上一份回执里的数，免得 A08 断一下又重报。
  这个沿用只进回执，健康第六节照旧只认本进程守卫证明过的。

重启起点（`status::resume`）：

| 上一份回执 | `last_aligned_at` | `deletes_blocked` |
|---|---|---|
| 没有（首次运行带本功能的发行、新建平面、恢复之后） | 平面打开的时刻 | `null` |
| 合法 | 沿用；晚于现在（时钟回拨）则 `null` | 沿用 |
| 读不出来（权限、链接、内容任一不对） | `null` | `null` |

## 4. 读法（F4）

169 探针 `probe.py` 每分钟读一次：逐段核父目录（属 root 或本人、不可组写，末段本人 0700），`O_NOFOLLOW` 打开，
文件本人 0600 单链接，按第 2 节闭合核对。读出两个布尔放进探针回执（字段由 5 个变成 7 个）：

| 布尔 | 为真的条件 |
|---|---|
| `sync_unaligned` | 读不出回执；或 `written_at` 在现在之后；或 `last_aligned_at` 为 `null`、在现在之后、距今满 6 小时（21600 秒） |
| `sync_deletes_blocked` | 回执读得出、`written_at` 距今不超过 15 分钟（900 秒）、且 `deletes_blocked` > 0 |

15 分钟是三个轮间隔：人侧卡住不再写回执时，「被挡」不能一直盖住停摆。
108 看门狗按 `watch_state.py` 出代号，优先级 A06 > A02 > A03 > A04 > **A08 > A07**，都要连续两分钟才报；
A07、A08 是 warning（Pushover 普通、Bark passive），发送成功后一天（86400 秒）内不重发，其余代号仍是 600 秒。
对外只发代号和可选的登录链接，不发个数、路径或时刻。

## 5. 兼容与安装顺序

- 人侧从 r20 起写回执；r19 及以前不认也不碰这个文件，r20 退回 r19 时它留在平面根原样不动。
- 探针新包配没写回执的人侧会一直报 A07：先上 r20，再装或升级 F4。
- 旧包（5 字段回执）和新包混装时，108 判 A01：两台必须装同一个包。

## 6. 用例

- `crates/local-server/tests/sync_plane_private/status.rs`：编解码两侧、对上判据、重启起点。
- `crates/local-server/tests/sync_plane_health/status.rs`：只在对上时前移、被挡沿用、真实目录里首装／重启／坏文件／不碰 `plane-state.json`。
- `crates/studiolo/tests/architecture_support/sync_plane.rs`：`recovery.rs` 的登记形态含两个回执固定名；`status.rs` 没有 IO。
- `crates/local-server/src/alert_main.rs`（`tests`）：发送程序只认登记的代号。
- `deploy/alerts/test_monitor.py`：读取两侧、6 小时与 15 分钟边界、优先级、按天重发、读不到失败关闭。
