//! 新根恢复的命令行 `studiolo-restore`（T-212 设计 4.6）：只在本机终端上由人执行。
//!
//! ```text
//! studiolo-restore run --config <私有配置> --checkpoint-copy <取回的副本> --checkpoint <封存标识>
//!     --seal-sha256 <64 位十六进制> --new-root <新根> --restore-id <恢复标识>
//!     [--resume] --confirm <检查名>（八项前置检查各一次）
//! studiolo-restore cutover begin|finish --config <私有配置> --new-root <新根> --restore-id <恢复标识>
//! ```
//!
//! 本程序不读写任何文件（本包只经各 owner 的入口碰磁盘），输入输出都走标准流：
//! - 标准输入（要从管道或文件接，不能是终端：种子在终端里敲会回显）：`run` 第一行是主人的 TOTP 种子，
//!   格式和 `studiolo-auth-admin restore` 一样是恰好 40 位小写十六进制，这里后面要跟一个换行（之后可能还接
//!   恢复记录）；不进参数、不进恢复记录。新开时只有这一行：恢复包没有秘密，从核验过的封存副本当场生成、
//!   立即兑换（一次性身份十分钟内有效）；接手（`--resume`）时种子之后接上一次的完整恢复记录；
//!   `cutover` 只有恢复记录。
//! - 标准输出：完整的恢复记录（第一行是头，之后每条一行，都是 JSON）。已有的先整份写出，新的每追加一条
//!   就写出并刷新，然后才做那一步。每次运行写到一个新文件，接手和切换时拿最新那一份。
//! - 标准错误：中文的进度、结论和下一步。
//!
//! 只认本机终端：标准错误要连着终端、标准输入不是终端，而且不在 SSH 会话里；不是的什么都不读、不写就拒绝。
//! 恢复标识每次恢复取一个新的：这份数据以前用过的（被封存的根本身是恢复出来的）一律拒绝。lineage 的上一环
//! 由数据定，不用给。
//! 退出码：0 做完；1 拒绝或没做完（原因和下一步在标准错误，新根照旧起不来）；2 用法不对。

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    os::fd::AsFd,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use studiolo_auth::TotpSeed;
use studiolo_backup::{
    checkpoint::staging::{RestoredCheckpoint, SealedReference},
    recovery::{
        kit::{BootstrapChannel, PreRestoreChecks, RecoveryGrant, RecoveryKit},
        transcript::{RecoveryTranscript, TranscriptEntry},
    },
};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};
use studiolo_host_config::Config;
use studiolo_local_server::checkpoint::{
    Cutover, CutoverRefusal, NewRootRestore, RestoreRefusal, RestoreReport, begin_cutover,
    finish_cutover, restore_into_new_root, restore_started,
};
use studiolo_vault_broker::valid_restore_id;
use zeroize::Zeroizing;

const USAGE: &str = "用法：
  studiolo-restore run --config <私有配置> --checkpoint-copy <取回的副本> --checkpoint <封存标识> \\
      --seal-sha256 <64位十六进制> --new-root <新根> --restore-id <恢复标识> \\
      [--resume] --confirm <检查名>（八项各一次）
  studiolo-restore cutover begin|finish --config <私有配置> --new-root <新根> --restore-id <恢复标识>
八项检查名：sync-adapter-disabled execution-adapter-disabled content-verified version-verified
  permissions-verified hashes-verified audit-continuity-verified space-verified
标准输入（从管道或文件接）：run 先一行 TOTP 种子，新开时就这一行，--resume 时再接上一次的恢复记录；
  cutover 只接恢复记录。
标准输出：完整的恢复记录，每次写到一个新文件（不要写回输入的那个文件）。";

/// 整次恢复共用的截止时间：大库要逐字节核对着放置，留足；到点没做完的，人接手再来。
const DEADLINE: Duration = Duration::from_secs(4 * 60 * 60);
/// 当场生成的恢复包里一次性身份的有效期：生成后马上兑换，用不了这么久。
const KIT_VALIDITY_MILLIS: i64 = 10 * 60 * 1000;
/// 标准输入里恢复记录的上限。
const INPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// 参数个数上限（`run` 全给也只有 32 个）。
const ARGUMENT_LIMIT: usize = 64;
/// 前置检查逐条要人确认的名字（`--confirm <名字>`），顺序和 `PreRestoreChecks` 的字段一一对应。
const CHECKS: [&str; 8] = [
    "sync-adapter-disabled",
    "execution-adapter-disabled",
    "content-verified",
    "version-verified",
    "permissions-verified",
    "hashes-verified",
    "audit-continuity-verified",
    "space-verified",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Run,
    Begin,
    Finish,
}

/// 三个子命令都要的：私有配置、新根、恢复标识。
#[derive(Debug, PartialEq, Eq)]
struct Target {
    config: PathBuf,
    new_root: PathBuf,
    restore_id: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Run {
    target: Target,
    copy: PathBuf,
    checkpoint: CheckpointId,
    seal_sha256: String,
    resume: bool,
    confirmed: BTreeSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run(Run),
    Begin(Target),
    Finish(Target),
}

/// 用法不对退出 2，拒绝或没做完退出 1；原因都是固定的中文，不带种子和底层错误。
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Usage(&'static str),
    Refused(String),
}

fn refused(reason: &str) -> Failure {
    Failure::Refused(reason.to_owned())
}

fn parse(args: &[String]) -> Result<Command, &'static str> {
    let (kind, rest) = match args {
        [command, rest @ ..] if command == "run" => (Kind::Run, rest),
        [command, step, rest @ ..] if command == "cutover" && step == "begin" => {
            (Kind::Begin, rest)
        }
        [command, step, rest @ ..] if command == "cutover" && step == "finish" => {
            (Kind::Finish, rest)
        }
        _ => return Err("子命令应为 run、cutover begin 或 cutover finish"),
    };
    let mut values = BTreeMap::new();
    let mut confirmed = BTreeSet::new();
    let mut resume = false;
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        let flag = flag.as_str();
        if kind == Kind::Run && flag == "--resume" {
            if resume {
                return Err("--resume 只能给一次");
            }
            resume = true;
            continue;
        }
        let known = matches!(flag, "--config" | "--new-root" | "--restore-id")
            || (kind == Kind::Run
                && matches!(
                    flag,
                    "--checkpoint-copy" | "--checkpoint" | "--seal-sha256" | "--confirm"
                ));
        if !known {
            return Err("有不认识的参数");
        }
        let value = rest.next().ok_or("参数后面缺值")?.clone();
        if flag == "--confirm" {
            if !CHECKS.contains(&value.as_str()) {
                return Err("--confirm 后面要是八项前置检查名之一");
            }
            if !confirmed.insert(value) {
                return Err("同一项检查确认了两次");
            }
        } else if values.insert(flag, value).is_some() {
            return Err("同一个参数给了两次");
        }
    }
    let mut take = |flag: &str, missing: &'static str| values.remove(flag).ok_or(missing);
    let target = Target {
        config: absolute(take("--config", "缺 --config")?)?,
        new_root: absolute(take("--new-root", "缺 --new-root")?)?,
        restore_id: identifier(take("--restore-id", "缺 --restore-id")?)?,
    };
    match kind {
        Kind::Begin => Ok(Command::Begin(target)),
        Kind::Finish => Ok(Command::Finish(target)),
        Kind::Run => {
            let seal_sha256 = take("--seal-sha256", "缺 --seal-sha256")?;
            if !studiolo_core::is_sha256_hex(&seal_sha256) {
                return Err("--seal-sha256 要恰好 64 位小写十六进制");
            }
            let checkpoint = CheckpointId::parse(take("--checkpoint", "缺 --checkpoint")?)
                .map_err(|_| "--checkpoint 不是合法的封存标识")?;
            let copy = absolute(take("--checkpoint-copy", "缺 --checkpoint-copy")?)?;
            Ok(Command::Run(Run {
                target,
                copy,
                checkpoint,
                seal_sha256,
                resume,
                confirmed,
            }))
        }
    }
}

fn absolute(value: String) -> Result<PathBuf, &'static str> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err("路径都要是绝对路径");
    }
    Ok(path)
}

/// 恢复标识：1～55 个字母、数字或 `-`（Broker lineage 和 Auth 撤权记录同一个规则）。
fn identifier(value: String) -> Result<String, &'static str> {
    if valid_restore_id(&value) {
        Ok(value)
    } else {
        Err("恢复标识要 1～55 个字母、数字或 -")
    }
}

fn checks(confirmed: &BTreeSet<String>) -> PreRestoreChecks {
    let has = |index: usize| confirmed.contains(CHECKS[index]);
    PreRestoreChecks {
        sync_adapter_disabled: has(0),
        execution_adapter_disabled: has(1),
        content_verified: has(2),
        version_verified: has(3),
        permissions_verified: has(4),
        hashes_verified: has(5),
        audit_continuity_verified: has(6),
        space_verified: has(7),
    }
}

fn channel() -> Result<BootstrapChannel, &'static str> {
    let ssh = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some());
    admitted(
        rustix::termios::isatty(io::stderr().as_fd()),
        rustix::termios::isatty(io::stdin().as_fd()),
        ssh,
    )
}

/// 恢复只认本机终端上的人：标准错误连着终端（人看得到进度和结论）；标准输入不是终端（种子和恢复记录经管道
/// 或文件给，在终端里敲种子会回显在屏幕上）；不在 SSH 会话里。SSH 只看环境变量：防的是误从远程登录上做，
/// 不防有意绕开的人；真正的门槛是取回副本要用的仓库口令、主人的 TOTP 种子和本机文件权限。
fn admitted(
    stderr_terminal: bool,
    stdin_terminal: bool,
    ssh: bool,
) -> Result<BootstrapChannel, &'static str> {
    if !stderr_terminal {
        return Err("只认本机终端：标准错误要连着终端");
    }
    if stdin_terminal {
        return Err("标准输入要从管道或文件接：种子和恢复记录不在终端里敲（会回显在屏幕上）");
    }
    if ssh {
        return Err("SSH 会话里不做恢复，要在这台机器自己的终端上做");
    }
    Ok(BootstrapChannel::LocalTty)
}

/// 标准输入第一行：主人的 TOTP 种子，恰好 40 位小写十六进制加换行。直接从描述符逐字节读，不经标准库的
/// 输入缓冲（那份缓冲不清零），读到的字节只放在零化容器里；后面的恢复记录照常经标准库读。
fn read_seed(reference: &str) -> Result<TotpSeed, Failure> {
    let malformed = || refused("标准输入第一行要是恰好 40 位小写十六进制的 TOTP 种子");
    let input = io::stdin();
    let mut line = Zeroizing::new([0_u8; 41]);
    let mut length = 0;
    loop {
        if length == line.len() {
            return Err(malformed());
        }
        match rustix::io::read(input.as_fd(), &mut line[length..=length]) {
            Ok(0) => return Err(malformed()),
            Ok(_) if line[length] == b'\n' => break,
            Ok(_) => length += 1,
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Err(refused("读不了标准输入")),
        }
    }
    if length != 40 {
        return Err(malformed());
    }
    TotpSeed::from_hex(reference.to_owned(), &line[..40]).map_err(|_| malformed())
}

/// 种子之后（或 `cutover` 的整个）标准输入：恢复记录，UTF-8 文本，不超过上限；新开时是空的。
fn read_rest() -> Result<String, Failure> {
    let mut text = String::new();
    io::stdin()
        .lock()
        .take(INPUT_LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|_| refused("标准输入读不了或不是 UTF-8 文本"))?;
    if text.len() as u64 > INPUT_LIMIT {
        return Err(refused("标准输入超过 16 MiB"));
    }
    Ok(text)
}

/// 恢复记录的一行写到标准输出并刷新；写不出去就停，编排不做那一步。
fn emit(line: &str) -> Result<(), ()> {
    let mut output = io::stdout().lock();
    writeln!(output, "{line}")
        .and_then(|()| output.flush())
        .map_err(drop)
}

fn emit_entry(entry: &TranscriptEntry) -> Result<(), ()> {
    emit(&RecoveryTranscript::entry_line(entry).map_err(drop)?)
}

/// 已有的整份记录先写出去：头一行，之后每条一行。
fn emit_all(transcript: &RecoveryTranscript) -> Result<(), Failure> {
    let failed = || refused("恢复记录写不到标准输出");
    emit(&transcript.header_line().map_err(|_| failed())?).map_err(|()| failed())?;
    for entry in &transcript.entries {
        emit_entry(entry).map_err(|()| failed())?;
    }
    Ok(())
}

/// 新开：从核验过的封存副本当场生成恢复包（一次性身份、短期有效），马上兑换，建立恢复记录（第一条是
/// 打开恢复包）。恢复包里没有秘密，所以不单独保存（T-212 设计第 10 节）。
fn opened(
    restored: &RestoredCheckpoint,
    channel: BootstrapChannel,
) -> Result<RecoveryGrant, Failure> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .ok_or_else(|| refused("系统时钟不可用"))?;
    let mut kit = RecoveryKit::for_restored(
        restored,
        format!("restore-{}", ulid::Ulid::generate()),
        InstantMillis::from_millis(now.saturating_add(KIT_VALIDITY_MILLIS)),
    );
    kit.redeem(channel, InstantMillis::from_millis(now))
        .map_err(|error| Failure::Refused(format!("恢复包打不开：{error}")))
}

/// 接手或切换：读回上一次的完整恢复记录，取得写入授权。
fn resumed(text: &str) -> Result<RecoveryGrant, Failure> {
    let transcript = RecoveryTranscript::from_lines(text)
        .map_err(|_| refused("恢复记录读不回来：格式不对，或完整性链断了"))?;
    let authorization = transcript
        .authorize_write()
        .map_err(|_| refused("恢复记录的第一条不是打开恢复包，取不到写入授权"))?;
    Ok(RecoveryGrant {
        transcript,
        authorization,
    })
}

fn sessions(target: &Target) -> Result<(Config, studiolo_auth::SessionConfig), Failure> {
    let config = Config::load(&target.config).map_err(|_| refused("私有配置读不了或不合法"))?;
    let sessions = config
        .sessions()
        .map_err(|_| refused("私有配置里的主人或会话期限不合法"))?;
    Ok((config, sessions))
}

fn restore(run: Run, channel: BootstrapChannel) -> Result<String, Failure> {
    let checks = checks(&run.confirmed);
    if !checks.is_complete() {
        let missing: Vec<&str> = CHECKS
            .into_iter()
            .filter(|name| !run.confirmed.contains(*name))
            .collect();
        return Err(Failure::Refused(format!(
            "前置检查还没逐条确认：{}",
            missing.join("、")
        )));
    }
    let (config, sessions) = sessions(&run.target)?;
    let seed = read_seed(config.seed_ref())?;
    let text = read_rest()?;
    if !run.resume && !text.trim().is_empty() {
        return Err(refused(
            "新开时标准输入只要一行种子；接手上一次要加 --resume，种子之后接上一次的恢复记录",
        ));
    }
    let deadline = Instant::now() + DEADLINE;
    let reference = SealedReference {
        checkpoint_id: run.checkpoint.clone(),
        seal_sha256: run.seal_sha256.clone(),
    };
    let restored = RestoredCheckpoint::open(&run.copy, &reference, deadline).map_err(|_| {
        refused("取回的副本核验不过：目录要只读、属于当前账号，封存摘要要和可信记录里的一样")
    })?;
    let mut grant = if run.resume {
        resumed(&text)?
    } else {
        opened(&restored, channel)?
    };
    if grant.transcript.checkpoint_id != run.checkpoint {
        return Err(refused("恢复记录不是这个封存的"));
    }
    emit_all(&grant.transcript)?;
    let request = NewRootRestore {
        restored: &restored,
        new_root: &run.target.new_root,
        restore_id: &run.target.restore_id,
        resume_interrupted: run.resume,
        checks,
        deadline,
        sessions,
        totp_seed: seed,
    };
    let restore_id = &run.target.restore_id;
    let result = restore_into_new_root(&mut grant, request, &mut emit_entry);
    let report = result.map_err(|refusal| {
        // 内存里的记录和写出去的一样（交不出去的那条已撤掉）：有这次的放置意图才可能写过新根。
        let started = restore_started(&grant.transcript, restore_id);
        Failure::Refused(restore_reason(refusal, started))
    })?;
    Ok(restored_summary(&report, restore_id))
}

fn cutover(target: &Target, finish: bool) -> Result<String, Failure> {
    let (_, sessions) = sessions(target)?;
    let mut grant = resumed(&read_rest()?)?;
    emit_all(&grant.transcript)?;
    let request = Cutover {
        new_root: &target.new_root,
        restore_id: &target.restore_id,
        sessions,
    };
    if finish {
        let facts = finish_cutover(&mut grant, request, &mut emit_entry)
            .map_err(|refusal| refused(cutover_reason(refusal)))?;
        return Ok(format!(
            "最终切换完成：主人已经在新根上重新登录，失效事实全部成立（签发代次 {}）。新根就是生产了；旧根留到回滚窗口结束再由人处理。",
            facts.issuance_generation
        ));
    }
    begin_cutover(&mut grant, request, &mut emit_entry)
        .map_err(|refusal| refused(cutover_reason(refusal)))?;
    Ok("已记下最终切换开始。下一步：把生产配置指到新根、起服务、主人重新登录并核对内容；之后停掉服务，接这份新的恢复记录执行 cutover finish。".into())
}

fn restored_summary(report: &RestoreReport, restore_id: &str) -> String {
    let owners: Vec<String> = report
        .restored_owners
        .iter()
        .map(|owner| format!("{owner:?}"))
        .collect();
    let invalidation = &report.invalidation;
    format!(
        "恢复做完：恢复标识 {restore_id}，恢复了 {}{}；Auth 身份版本 {}，撤掉 {} 个会话、{} 个 L3、{} 条恢复码{}。下一步：核对新根内容，再接这份新的恢复记录执行 cutover begin。",
        owners.join("、"),
        if report.reused_placement {
            "（接手上一次，已放好的没有重写）"
        } else {
            ""
        },
        invalidation.epoch,
        invalidation.revoked_sessions,
        invalidation.cleared_l3,
        invalidation.invalidated_recovery_codes,
        if report.sync_left_closed {
            "；双向同步留在关闭，要按引导流程从零重新打开"
        } else {
            ""
        },
    )
}

/// 拒绝原因加下一步。`started`：恢复记录里有这次恢复的放置意图，新根可能已经写过，只能接着这份记录接手；
/// 没有的话这次一个字节都没往新根写，改正之后再来就行。恢复标识用过的例外：接手解决不了，要从头来。
fn restore_reason(refusal: RestoreRefusal, started: bool) -> String {
    let reason = match refusal {
        RestoreRefusal::RestoreId => "恢复标识不合法".to_owned(),
        RestoreRefusal::RestoreIdUsed => {
            return "这个恢复标识这份数据以前用过（被封存的根本身是恢复出来的），再用一次启动闸门会认旧的撤权记录。新根照旧起不来（Auth 还在 auth.held 下）；删掉这次放进新根的东西，换一个没用过的恢复标识、不接恢复记录从头来。".to_owned();
        }
        RestoreRefusal::Transcript => {
            "恢复记录对不上：接手要接上一次那份完整记录（新开时不接记录）；或者恢复记录写不到标准输出".to_owned()
        }
        RestoreRefusal::ChecksIncomplete => "前置检查没有全部确认".to_owned(),
        RestoreRefusal::Roster => "封存的 owner 名单或水位不成套，这一版不恢复".to_owned(),
        RestoreRefusal::AlreadyPlaced => {
            "新根里已经有东西：新开要用空的新根，接手上一次要加 --resume".to_owned()
        }
        RestoreRefusal::CleanupRequired => {
            "新根里留着上一次放置中途失败的 <名字>.restoring：人看过、清掉再来".to_owned()
        }
        RestoreRefusal::Placement => {
            "封存内容没能逐字节放进新根、接手时已放好的组件和封存对不上，或者新根里 auth、agent 的正式名和暂用名（.held）同时在".to_owned()
        }
        RestoreRefusal::Rebind => {
            "Broker 的新根重绑定没有完成，或者新根 data 里的 lineage 和这次恢复对不上".to_owned()
        }
        RestoreRefusal::Audit => {
            "账本打不开，或者链尾和清单水位对不上（水位之后只许有这次恢复 Auth 撤权自己记的事件）".to_owned()
        }
        RestoreRefusal::Owner(owner) => {
            format!("{owner:?} 的失效没有做完（核对、失效、事后核对或改正式名有一步不过）")
        }
        RestoreRefusal::Auth => {
            "Auth 撤权没有完成（TOTP 种子不对也会这样；改不成正式名 auth 也算）".to_owned()
        }
    };
    let next = if started {
        "新根照旧起不来；看过原因之后，接这次写出的恢复记录加 --resume 接手"
    } else {
        "这次没往新根写任何东西，新根照旧起不来；改正之后再来"
    };
    format!("{reason}。{next}。")
}

fn cutover_reason(refusal: CutoverRefusal) -> &'static str {
    match refusal {
        CutoverRefusal::Transcript => "恢复记录对不上，或者写不到标准输出",
        CutoverRefusal::Sequence => "次序不对：恢复做完才能开始切换，记过开始才能完成，各只做一次",
        CutoverRefusal::NewRoot => "新根对不上、打不开（新根上的服务要先停），或者启动闸门不放行",
        CutoverRefusal::NotReauthenticated => {
            "主人还没在新根上重新登录过：起服务登录一次，停掉服务再来"
        }
        CutoverRefusal::Invalidation => "失效事实核对不过",
    }
}

fn execute() -> Result<String, Failure> {
    let args = std::env::args_os()
        .skip(1)
        .take(ARGUMENT_LIMIT + 1)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| Failure::Usage("参数编码无效"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if args.len() > ARGUMENT_LIMIT {
        return Err(Failure::Usage("参数太多"));
    }
    let command = parse(&args).map_err(Failure::Usage)?;
    let channel = channel().map_err(refused)?;
    // 种子读进内存之前先收紧秘密驻留面，和服务器、离线恢复工具同一份实现。
    let residency = studiolo_process_safety::harden().map_err(|_| refused("秘密驻留面加固失败"))?;
    writeln!(io::stderr(), "秘密驻留面：{residency}").map_err(|_| refused("标准错误写不出去"))?;
    match command {
        Command::Run(run) => restore(run, channel),
        Command::Begin(target) => cutover(&target, false),
        Command::Finish(target) => cutover(&target, true),
    }
}

fn main() -> ExitCode {
    match execute() {
        Ok(summary) => {
            eprintln!("{summary}");
            ExitCode::SUCCESS
        }
        Err(Failure::Usage(reason)) => {
            eprintln!("用法不对：{reason}\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Refused(reason)) => {
            eprintln!("没有做完：{reason}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn full_run() -> Vec<String> {
        let mut values = args(&[
            "run",
            "--config",
            "/owner/owner.json",
            "--checkpoint-copy",
            "/fetched/copy",
            "--checkpoint",
            "cp-0001",
            "--seal-sha256",
            &"ab".repeat(32),
            "--new-root",
            "/restored",
            "--restore-id",
            "drill-1",
        ]);
        for name in CHECKS {
            values.extend(args(&["--confirm", name]));
        }
        values
    }

    #[test]
    fn run_takes_every_check_by_name_and_the_fixed_inputs() {
        let Command::Run(run) = parse(&full_run()).unwrap() else {
            panic!("应为 run");
        };
        assert_eq!(run.target.config, PathBuf::from("/owner/owner.json"));
        assert_eq!(run.target.new_root, PathBuf::from("/restored"));
        assert_eq!(run.target.restore_id, "drill-1");
        assert_eq!(run.copy, PathBuf::from("/fetched/copy"));
        assert_eq!(run.checkpoint, CheckpointId::parse("cp-0001").unwrap());
        assert!(!run.resume);
        assert!(checks(&run.confirmed).is_complete());

        let mut resumed = full_run();
        resumed.extend(args(&["--resume"]));
        let Command::Run(run) = parse(&resumed).unwrap() else {
            panic!("应为 run");
        };
        assert!(run.resume);

        // 少确认一项：参数本身合法，但前置检查不完整，运行时在读任何输入之前拒绝。
        let mut partial = full_run();
        partial.truncate(partial.len() - 2);
        let Command::Run(run) = parse(&partial).unwrap() else {
            panic!("应为 run");
        };
        assert!(!checks(&run.confirmed).is_complete());
        assert!(!run.confirmed.contains("space-verified"));
    }

    #[test]
    fn cutover_steps_take_only_the_shared_inputs() {
        let shared = [
            "--config",
            "/owner/owner.json",
            "--new-root",
            "/restored",
            "--restore-id",
            "drill-1",
        ];
        for (step, expected) in [("begin", Kind::Begin), ("finish", Kind::Finish)] {
            let mut values = args(&["cutover", step]);
            values.extend(args(&shared));
            let command = parse(&values).unwrap();
            let target = match (expected, command) {
                (Kind::Begin, Command::Begin(target)) | (Kind::Finish, Command::Finish(target)) => {
                    target
                }
                _ => panic!("子命令认错了"),
            };
            assert_eq!(target.restore_id, "drill-1");
            // 只有 run 才收的参数，切换时给了就是用法不对。
            for extra in [&["--resume"][..], &["--confirm", "space-verified"][..]] {
                let mut wrong = values.clone();
                wrong.extend(args(extra));
                assert!(parse(&wrong).is_err());
            }
        }
    }

    #[test]
    fn malformed_arguments_are_usage_errors() {
        let replace = |flag: &str, value: &str| {
            let mut values = full_run();
            let index = values.iter().position(|item| item == flag).unwrap();
            values[index + 1] = value.to_owned();
            values
        };
        let mut duplicate_check = full_run();
        duplicate_check.extend(args(&["--confirm", "space-verified"]));
        let mut duplicate_flag = full_run();
        duplicate_flag.extend(args(&["--new-root", "/elsewhere"]));
        let mut twice_resumed = full_run();
        twice_resumed.extend(args(&["--resume", "--resume"]));
        let mut dangling = full_run();
        dangling.push("--restore-id".into());
        for wrong in [
            args(&[]),
            args(&["restore"]),
            args(&["cutover", "abort"]),
            replace("--config", "owner.json"),
            replace("--new-root", "restored"),
            replace("--checkpoint-copy", "copy"),
            replace("--restore-id", "drill 1"),
            replace("--restore-id", &"a".repeat(56)),
            replace("--seal-sha256", &"AB".repeat(32)),
            replace("--seal-sha256", "abc"),
            replace("--checkpoint", " "),
            replace("--confirm", "everything"),
            duplicate_check,
            duplicate_flag,
            twice_resumed,
            dangling,
            // 上一环由数据定，命令行不再收（以前的 --parent-lineage）。
            {
                let mut values = full_run();
                values.extend(args(&["--parent-lineage", "drill-0"]));
                values
            },
            {
                let mut values = full_run();
                values.push("--force".into());
                values
            },
        ] {
            assert!(parse(&wrong).is_err(), "{wrong:?} 应当是用法不对");
        }
        for missing in [
            "--config",
            "--new-root",
            "--restore-id",
            "--checkpoint",
            "--seal-sha256",
            "--checkpoint-copy",
        ] {
            let mut values = full_run();
            let index = values.iter().position(|item| item == missing).unwrap();
            values.drain(index..=index + 1);
            assert!(parse(&values).is_err(), "缺 {missing} 应当是用法不对");
        }
    }

    #[test]
    fn only_a_person_at_this_terminal_with_piped_input_is_admitted() {
        assert_eq!(admitted(true, false, false), Ok(BootstrapChannel::LocalTty));
        for (stderr, stdin, ssh, expected) in [
            (false, false, false, "只认本机终端"),
            (false, true, true, "只认本机终端"),
            (true, true, false, "标准输入要从管道或文件接"),
            (true, true, true, "标准输入要从管道或文件接"),
            (true, false, true, "SSH"),
        ] {
            let reason = admitted(stderr, stdin, ssh).unwrap_err();
            assert!(reason.contains(expected), "{reason}");
        }
    }

    #[test]
    fn every_restore_refusal_says_the_new_root_stays_closed_and_what_to_do_next() {
        for refusal in [
            RestoreRefusal::RestoreId,
            RestoreRefusal::Transcript,
            RestoreRefusal::ChecksIncomplete,
            RestoreRefusal::Roster,
            RestoreRefusal::AlreadyPlaced,
            RestoreRefusal::CleanupRequired,
            RestoreRefusal::Placement,
            RestoreRefusal::Rebind,
            RestoreRefusal::Audit,
            RestoreRefusal::Owner(studiolo_core::checkpoint::CheckpointOwner::Control),
            RestoreRefusal::Auth,
        ] {
            let started = restore_reason(refusal, true);
            assert!(started.contains("新根照旧起不来"), "{started}");
            assert!(started.ends_with("加 --resume 接手。"), "{started}");
            let untouched = restore_reason(refusal, false);
            assert!(untouched.contains("没往新根写任何东西"), "{untouched}");
            assert!(untouched.ends_with("改正之后再来。"), "{untouched}");
        }
        // 恢复标识用过的：接手解决不了，不管写没写过都叫人换标识从头来。
        for started in [true, false] {
            let reason = restore_reason(RestoreRefusal::RestoreIdUsed, started);
            assert!(reason.contains("新根照旧起不来"), "{reason}");
            assert!(reason.contains("换一个没用过的恢复标识"), "{reason}");
            assert!(!reason.contains("--resume"), "{reason}");
        }
    }
}
