//! 执行器必须消费真实封存能力；不能把参数校验改回任意有效路径。
use super::super::authority_tokens::compact;
use std::path::Path;
pub(super) fn fixed_binding(path: &Path, raw: &str) -> bool {
    let source = compact(raw);
    let anchors: &[&str] = if path.ends_with("crates/backup/src/checkpoint/staging/io/journal.rs") {
        &[
            "LIMIT:usize=16*1024",
            "Self::Identity=>(\"identity.json\",\"identity.new\")",
            "Self::Export=>(\"export.json\",\"export.new\")",
            "Self::Pending=>(\"pending.json\",\"pending.new\")",
            "Self::Sealed=>(\"sealed.json\",\"sealed.new\")",
            "Regular::create(self,temporary,bytes.len()asu64)?",
            "self.publish(temporary,name)?;",
            "file.require_readonly()?;",
            "file.check(self,name)?;",
        ]
    } else if path.ends_with("crates/backup/src/checkpoint/staging/housekeeping.rs") {
        &[
            "parent.acquire_existing_lease()?;",
            "plan::read_all(&parent,reader,started,deadline)?;",
            "parent.check_path(parent_path)?;",
            "plan.check(&parent)?;",
            "expiry>now",
            // 原根和登记目录的每一步删除都记在同一份预算里；登记目录先改成带原 inode 的「删除中」名字再删。
            "root.discard_within(&parent,name,reader,budget)?;",
            "plan.marker.retire(&parent,&plan.name,&plan.removing,last,budget)?;",
            "letlast=JournalFile::Identity.names().0;",
        ]
    } else if path.ends_with("crates/backup/src/checkpoint/staging/housekeeping/plan.rs") {
        &[
            "identity.validate(parent,&marker,key,reader,now)?;",
            "parent.reclaim_child(&root_name,proof.directory,reader)?;",
            "ifknown!=covered",
            "self.marker.read_journal(*kind)?!=*expected",
            "ifretained&&prefix!=\"sealed\"",
            // 删到一半的登记只按名字里写的原 inode 认，身份记录只在别的都删完之后才许缺。
            "ifmarker.identity().1!=inode{returnErr(StagingError::Changed);}",
            "Noneiffiles.is_empty()=>{}None=>returnErr(StagingError::CleanupRequired)",
        ]
    } else if path.ends_with("crates/backup-worker/src/source.rs") {
        &[
            "config:&WorkerConfig,key:JobKey,reference:&SealedReference",
            "key.checkpoint_id!=reference.checkpoint_id",
            "SealedCheckpoint::open_for_worker(config.sealed_parent(),reference,config.reader_identity(),now,deadline)",
            "SealedCheckpoint::open_for_worker(&self.parent,self.sealed().reference(),self.identity,utc_now()?,self.deadline())",
            "self.binding.check_reopened(&reopened)?;",
            "self.check_parents()?;self.check_time()",
            "self.deadline())?!=self.parents",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner.rs") {
        &[
            "pubfnnew(job:&ApprovedJob<'_>,credentials:ResticCredentials",
            "job.check_live()?;letsource=Source::open(job.config(),job.key().clone(),job.fact().source(),job.deadline()",
            "job.fact().verify_sealed(source.sealed())",
            "Runtime::open(job.config(),credentials,source.deadline())?",
            "self.source.accepts(invocation)?;self.source.check()?;",
            "letsource_time=||self.source.check_time();",
            "self.source.deadline(),&source_time",
            "self.source.check()?;parse_output(&output)",
        ]
    } else if path.ends_with("crates/backup/src/worker/repository_check.rs") {
        // 抽查窗口只定在这一处：worker 的截止时间、生产者补「中途没了」的时限、健康页正文都从它算。
        &["pubconstCHECK_WINDOW:Duration=Duration::from_secs(20*60);"]
    } else if path.ends_with("crates/backup-worker/src/repository_check.rs") {
        // 抽查只跑派下的那一组、只用追加身份、最多 20 分钟（共用包的窗口，写全路径，本地同名常量顶替不了）；
        // 清理没能证实必须退出，不能归成没做完。
        &[
            "letdeadline=Instant::now()+studiolo_backup::worker::repository_check::CHECK_WINDOW;",
            "&ResticCommand::CheckSubset{bucket:assignment.key.bucket}",
            "CredentialHolder::BackupWorker,ResticIdentity::Append",
            "runtime.check_subset(invocation.argv(),interrupt)",
            "ProcessError::CleanupFailed=>returnChecked::Shutdown",
            "ProcessError::Interrupted=>IncompleteReason::Preempted",
        ]
    } else if path.ends_with("crates/backup-worker/src/serve.rs") {
        // 作业优先：没有待办时才轮到抽查，抽查遇到待办作业就让路；清理没能证实就退出。
        &[
            "Ok(None)=>returnidle(residency,config,held)",
            "ifletSome(pending)=held.take(){returnOk(deliver(config,pending,held));}",
            "Ok(Some(_))=>Err(ProcessError::Interrupted)",
            "Checked::Shutdown=>returnErr(ServeError::Shutdown)",
            "YIELD_POLL:Duration=Duration::from_secs(30);",
        ]
    } else if path.ends_with("crates/backup-worker/src/job.rs") {
        &[
            "pubstructApprovedJob<'a>{config:&'aWorkerConfig,audit:WorkerAuditClient<'a>,fact:AdmissionFact,deadline:Instant}",
            "WorkerAuditClient::new(config,key,BackupTargetKind::DedicatedAppendOnlyTarget,deadline)",
            "audit.admitted_source()",
            "self.fact.check_live()",
        ]
    } else if path.ends_with("crates/backup-worker/src/source/binding.rs") {
        &[
            "key.checkpoint_id!=reference.checkpoint_id||sealed.reference()!=reference",
            "tag:JobTag::for_job(&key)",
            "source_root:sealed.ticket().transfer_source.root.clone()",
            "ifinvocation==&self.backup{Ok(())}else{Err(ProcessError::Configuration)}",
            "reopened.reference()!=self.sealed.reference()||reopened.ticket()!=self.sealed.ticket()||reopened.manifest()!=self.sealed.manifest()",
            "reopened.inventory()!=self.sealed.inventory()",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/inspect.rs") {
        // BUG-053：整份封存只起一个 restic 读回（对封存根一次 `dump --archive tar`），期望照清单逐个目录、
        // 逐个文件（长度和摘要）造；改回一个文件一个 `dump`，1 万多个文件在封存副本到期前核不完。
        &[
            "self.source.check()?;verify_files(self.source.repository(),snapshot,&self.sealed().ticket().transfer_source.root,self.sealed().inventory(),",
            "self.runtime.verify_tree(argv,expected,self.source.deadline(),",
            "self.source.check_time()",
            "letargv=plan::tree(repository,snapshot,root)?;letmutexpected=process::TreeExpectation::new(root)?;",
            "fordirectoryininventory.directories(){expected.directory(directory)?;}",
            "forfileininventory.files(){expected.file(file.path().as_str(),file.bytes(),file.sha256())?;}run(&argv,&expected)",
            "self.source.check()}",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/probe.rs") {
        &[
            "ifkey!=self.key()",
            "self.locate_snapshot(self.key())?.as_ref()!=Some(snapshot)",
            "self.verify_snapshot_files(snapshot)?;",
            "inventory:self.sealed().inventory()",
            "letverified_at=self.source.current_time()?;",
            "self.runtime.capture(",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/inspect/plan.rs") {
        &[
            "snapshot.as_str().len()!=64",
            "lettext=root.to_str().ok_or(ProcessError::Configuration)?;if!root.is_absolute()",
            "text.split('/').any(|part|matches!(part,\".\"|\"..\"))",
            "\"snapshots\".into(),\"--tag\".into(),tag.as_str().into()",
            "\"dump\".into(),\"--archive\".into(),\"tar\".into(),snapshot.as_str().into(),text.into()",
            "\"ls\".into(),snapshot.as_str().into()",
        ]
    } else if path.ends_with("crates/backup-worker/src/runtime.rs") {
        &[
            "ifself.poisoned.get(){returnErr(ProcessError::CleanupFailed);}",
            "self.poisoned.set(true)",
            "self.window.check()?;source_time()",
            "self.temporary.check()",
            "ResticPackage::open(config.restic_sha256(),deadline)?",
            "process::run_check(&self.package,&self.arguments(argv),&self.credentials,self.temporary.path(),self.window.deadline(),&||self.window.check(),interrupt)",
            // T-216：批准的仓库 CA 从配置一路带到每一次连仓库的 restic 调用，哪一环断了都判红
            // （`with_ca` 本身的行为由 runtime.rs 的单测守）。`runtime.rs` 怎么用 `process` 模块另由
            // `runtime_process_use` 钉住：六处调用（下面这处不连仓库的 `version`，加上连仓库的五处）之外不许再有。
            "letversion=process::run_scoped(&package,&[\"version\".into()],None,temporary.path(),deadline.min(started+Duration::from_secs(10)),&||window.check())?;",
            "repository_ca:config.repository_ca()",
            "fnarguments(&self,argv:&[String])->Vec<String>{with_ca(self.repository_ca,argv)}",
            "letargv=self.arguments(argv);letresult=matchinput{Some(bytes)=>process::run_input(&self.package,&argv,&self.credentials,self.temporary.path(),deadline,&check,bytes),None=>process::run_scoped(&self.package,&argv,Some(&self.credentials),self.temporary.path(),deadline,&check)};",
            "process::verify_digest(&self.package,&self.arguments(argv),&self.credentials,self.temporary.path(),deadline.min(self.window.deadline()),&check,expected)",
            "process::verify_tree(&self.package,&self.arguments(argv),&self.credentials,self.temporary.path(),deadline.min(self.window.deadline()),&check,expected)",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/final_record.rs") {
        &[
            "pubfnnew(job:&ApprovedJob<'_>,credentials:ResticCredentials",
            "self.admission.verify_draft(draft)",
            "draft.source_root()!=self.source_root",
            "self.source.check()?;",
            "self.verified.borrow_mut().take().ok_or(ProcessError::Source)?",
            "if&verified!=draft",
            "Some(&bytes),self.source.deadline(),&||self.source.check_time()",
            "read_remote(&self.runtime,draft)",
            "runtime.digest(argv,expected,runtime.deadline(),&||Ok(()))",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/final_record/read.rs") {
        &[
            "snapshot::locate_named(&output.stdout,&marker.tag,&marker.path)",
            "listing::verify(&output.stdout,&marker,&found,encoded.len()asu64)?",
            "DigestExpectation{bytes:encoded.len()asu64,sha256:&hash}",
            "iflocate()?.as_ref()!=Some(&found)",
        ]
    } else if path.ends_with("crates/backup-worker/src/runner/final_record/plan.rs") {
        &[
            "format!(\"{}.final\",original.as_str())",
            "format!(\"/studiolo-final-{}.json\",original.as_str())",
            "\"backup\".into(),\"--stdin\".into(),\"--stdin-filename\".into()",
        ]
    } else if path.ends_with("crates/backup-worker/src/source/window.rs") {
        &[
            "SystemTime::now().duration_since(UNIX_EPOCH)",
            "self.closed.get()||now<self.last.get()||now>=self.expiry||instant>=self.deadline.get()",
            "self.closed.set(true);",
            "self.deadline.set(self.deadline.get().min(expiry))",
        ]
    } else {
        &[]
    };
    anchors.iter().all(|anchor| source.contains(anchor))
        && (!path.ends_with("crates/backup-worker/src/runtime.rs") || runtime_process_use(raw))
}

/// `runtime.rs` 里引用 `process` 模块函数的地方：不连仓库的 `version` 一处，连仓库的五处，
/// 六处都在上面逐处钉住（连仓库的钉成带批准的 CA）。
const RUNTIME_PROCESS_FUNCTIONS: usize = 6;
/// `runtime.rs` 里 `process::` 一共出现几次：上面六处，加四处类型
/// （`Output` 两处、`DigestExpectation` 一处、`TreeExpectation` 一处）。
const RUNTIME_PROCESS_PATHS: usize = 10;

/// `runtime.rs` 多出一处没钉的 restic 调用，就可能不带 `--cacert` 去连自签证书的仓库（T-216）。
/// 按去掉行注释后的源码数，注释和文档里提到 `process::名字(` 不算：
/// - 引用 `process::` 里函数的地方恰好六处：调用、拿函数指针、`use crate::process::run_scoped` 这样直接引入都算一处；
/// - `process::` 的出现总数钉死：花括号或 `*` 引入会多出一次；
/// - 不许给模块起别名（`process as p`），起了别名上面两条就数不到。
///
/// 只管 `runtime.rs` 这一个文件、防手滑：别的文件直接调 `process` 里的函数不归这里管（现在没有这样的调用）。
fn runtime_process_use(raw: &str) -> bool {
    let code = compact(&without_line_comments(raw));
    let functions = code
        .match_indices("process::")
        .filter(|(at, token)| {
            code[at + token.len()..].starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        })
        .count();
    functions == RUNTIME_PROCESS_FUNCTIONS
        && code.matches("process::").count() == RUNTIME_PROCESS_PATHS
        && !code.contains("processas")
}

/// 每行从 `//` 起截掉（行注释、文档注释）。不认字符串：字符串里带 `//` 的那一行，后半行的代码也跟着截掉、
/// 数不到。`runtime.rs` 现在没有这样的行；这条守卫防手滑，不防故意绕。
fn without_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}
