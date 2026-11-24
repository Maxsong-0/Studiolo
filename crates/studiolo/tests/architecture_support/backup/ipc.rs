//! 备份审计客户端的路径、peer、作业绑定不可泛化成任意 socket 或通用账本代理。
use super::super::authority_tokens::{checked_closed, compact};
use std::path::Path;
pub(super) fn fixed_binding(path: &Path, source: &str) -> bool {
    let source = compact(source);
    let anchors: &[&str] = if path.ends_with("crates/local-server/src/backup/audit.rs") {
        &[
            "pubfnnew(admission:BackupAdmission)->Self",
            "self.admission.verify()?;",
            "self.admission.ledger.enter_operation()?",
            "request.key()!=self.admission.key()",
            "request.target()!=self.admission.target()",
            "Some(self.admission.receipt()?)",
        ]
    } else if path.ends_with("crates/local-server/src/backup/admission.rs") {
        &[
            "sealed:&SealedCheckpoint",
            "record.check_live()?;",
            "verify_source(&ledger,&record)?;",
            "letreceipt=ledger.append(event.clone())?;",
            "admission.verify()?;",
            "intent.event()!=&expected||intent.offset()>record.audit_offset()",
            "tail.offset()!=record.audit_offset()||tail.record_hash().to_hex()!=record.audit_hash()",
            "receipt.event()!=expected||receipt.offset()<=cutoff",
        ]
    } else if path.ends_with("crates/local-server/src/backup/producer.rs") {
        // 生产者只认固定目标和批准的 worker 账号；带键的请求每次都从账本重开原准入。
        // 队列命令、抽查结果上报和带键请求三条路各核一次目标，少一处都不行。
        if source
            .matches("ifrequest.target()!=TARGET{returnErr(AuditError::Serialization);}")
            .count()
            != 3
        {
            return false;
        }
        &[
            "self.plan.worker_uid==rustix::process::geteuid().as_raw()",
            "AuditSocket::new(stream,self.plan.worker_uid,deadline)",
            "ifrequest.target()!=TARGET{returnErr(AuditError::Serialization);}",
            "BackupAdmission::resume(self.ledger.clone(),request.key(),TARGET)",
            "ifqueue.outstanding(){returnOk((QueueAnswer::Outstanding,None));}",
            // 该不该封存：生产入口只用真实时钟，到没到间隔在发序号之前判断（BUG-057）。
            "fnschedule(self:&Arc<Self>)->Result<QueueAnswer,AuditError>{self.schedule_with(now_millis)}",
            "if!due(now,interval,queue.last_attempt_ms()){returnOk((QueueAnswer::NotDue,None));}",
            "self.ledger.append(backup_intent(&key,TARGET).map_err(|_|AuditError::Serialization)?)?;",
            // 冻结窗口只定格（BUG-045）：放开全部屏障之后马上标记窗口已过（关停只等到这里），
            // 定格的连接和句柄关掉之后才写准入。
            // 带分步用时的写法（BUG-050）：放开和标记窗口已过之间、定格关掉和写准入之间都不许插别的。
            "letmutreleased=frozen.release_timed(deadline,&mutsteps).map_err(|error|self.stuck(&error,&steps))?;self.window.close();",
            "drop(released);letsealed=sealed.map_err(|failure|after_release(failure,&after))?;BackupAdmission::approve(self.ledger.clone(),key.clone(),TARGET,&sealed)",
            "QueueCommand::RepositoryCheck=>self.repository_check()?",
            "ifletOk(request)=RepositoryCheckReport::decode(bytes){ifrequest.target()!=TARGET{returnErr(AuditError::Serialization);}returnself.report_check(&request);}",
        ]
    } else if path.ends_with("crates/local-server/src/backup/producer/check.rs") {
        // 抽查只在空闲时派：在封存、有作业没收口、上一组还在查都不派。挂满时限的原意图先由生产者补
        // 「中途没了」，生产入口只用真实时钟。结果要在账本里找到同一检查键、本目标的原意图才落账；
        // 原意图只取一条，多了就是账本坏了。
        &[
            "pub(super)fnrepository_check(&self)->Result<QueueAnswer,AuditError>{self.repository_check_with(now_millis)}",
            "ifstate.busy{returnOk(QueueAnswer::Outstanding);}",
            "ifstate.queue.outstanding(){returnOk(QueueAnswer::Outstanding);}",
            "letabandoned=state.queue.checks().abandoned(now);forkeyin&abandoned{self.ledger.append(check_abandoned(key).map_err(|_|AuditError::Serialization)?)?;}",
            "ifchecks.running(now).is_some(){returnOk(QueueAnswer::Outstanding);}",
            "ifrecord.event()!=&expected{returnErr(AuditError::CorruptStore);}",
            ".check_intent_record(&event)?.ok_or(AuditError::Unavailable)?;",
            "ifassignment.key==*request.key()&&target==TARGET",
            "if!same{returnErr(AuditError::CorruptStore);}",
            ".with_phase(AuditPhase::Intent).with_limit(2)",
            "ifrecords.len()>1{returnErr(AuditError::CorruptStore);}",
        ]
    } else if path.ends_with("crates/backup/src/worker/audit/report.rs") {
        // 上报只能导出唯一一条结果事件，「中途没了」只能由生产者补；回执要绑定请求摘要和这条事件。
        &[
            "#[serde(deny_unknown_fields)]",
            "self.0.outcome.reason()==Some(IncompleteReason::Abandoned)",
            "&CheckRecorder::Worker(self.0.repository.clone())",
            "reply.request_sha256!=codec::digest(&self.encode()?)",
            "self.check_record(&reply.record)?;",
        ]
    } else if path.ends_with("crates/local-server/src/backup/listener.rs") {
        PRODUCER_SOCKET
    } else if path.ends_with("crates/backup/src/worker/audit/admission.rs") {
        &[
            "#[serde(deny_unknown_fields)]",
            "self.key.checkpoint_id!=self.source.checkpoint_id",
            "self.audit_offset==0",
            "receipt.event()!=&record.event()?||receipt.offset()<=record.audit_offset",
            "now<self.content_cutoff||now>=self.expires_at",
        ]
    } else if path.ends_with("crates/backup-worker/src/audit.rs") {
        &[
            "identity(config)?;",
            "Endpoint::bind(config,deadline)?",
            "self.endpoint.connect(deadline)?",
            "self.endpoint.check(deadline)?;",
            "request.decode_reply(&response)",
            "BackupAuditRequest::append(self.key.clone(),self.target,event)",
            "filter.limit()!=Some(1)",
            "filter.event_id()!=Some(intent.event_id())",
            "self.exchange(BackupAuditRequest::admission(self.key.clone(),self.target))?",
            "AdmissionFact::from_receipt(&record)",
        ]
    } else if path.ends_with("crates/backup/src/worker/audit.rs") {
        &[
            "self.event().is_none()&&!self.is_admission()",
            "AdmissionFact::from_receipt(record)",
            "fact.key()==self.key()&&fact.target()==self.target()",
        ]
    } else if path.ends_with("crates/backup-worker/src/audit/endpoint.rs") {
        if source.matches("net::connect(").count() != 1
            || source.matches("net::socket_with(").count() != 1
        {
            return false;
        }
        &[
            "path=config.producer_socket().to_owned()",
            "self.proof.same(&Proof::open(&self.path,self.producer,self.group,deadline)?)",
            "self.check(deadline)?;letsocket=connect(&self.path,self.producer,deadline)?;self.check(deadline)?;",
            "SocketFlags::NONBLOCK|SocketFlags::CLOEXEC",
            "AuditSocket::new(UnixStream::from(fd),producer,deadline)",
        ]
    } else if path.ends_with("crates/backup/src/worker/audit/transport.rs") {
        if source.matches("self.poisoned=true;").count() != 2 {
            return false;
        }
        &[
            "self.poisoned=true;",
            "ifsize==0||size>AUDIT_MESSAGE_MAX_BYTES",
            "ifself.read_some(&mutextra)?!=0",
            "socket_peercred(&self.stream)",
            "!=self.peer_uid",
            "SocketType::STREAM",
            "AddressFamily::UNIX",
            "deadline:deadline.min(Instant::now()+Duration::from_secs(10))",
            "self.stream.shutdown(Shutdown::Write)",
        ]
    } else {
        &[]
    };
    anchors.iter().all(|anchor| source.contains(anchor))
}
/// 备份 socket 的条件要和 worker 一侧 endpoint proof 的核对一一对应：父目录 0750、socket 0660、读取组。
const PRODUCER_SOCKET: &[&str] = &[
    "held.st_uid!=uid||held.st_gid!=reader_gid||held.st_mode&0o7777!=0o750",
    "if!matches!(node.stat(),Err(Errno::NOENT)){returnErr(ListenError::Configuration);}",
    "FileType::from_raw_mode(stat.st_mode)!=FileType::Socket||stat.st_uid!=uid||stat.st_nlink!=1",
    "identity(&ready)!=claim.identity||ready.st_gid!=reader_gid||ready.st_mode&0o7777!=0o660",
    ".is_ok_and(|stat|identity(&stat)==self.identity)",
];
/// 主服务里只有这一个文件能建备份 socket：一次绑定、一次改属组、一次改权限、一次清理，都相对原父目录句柄。
pub(super) fn producer_socket(path: &Path, source: &str) -> String {
    if !path.ends_with("crates/local-server/src/backup/listener.rs") {
        return source.to_owned();
    }
    checked_closed(
        source,
        PRODUCER_SOCKET,
        &[
            (
                "fs::{AtFlags,FileType,Mode,OFlags,Stat,chmodat,chownat,fstat,open,statat,unlinkat}",
                1,
            ),
            (
                "open(directory,OFlags::RDONLY|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty())",
                1,
            ),
            ("fstat(&parent)", 1),
            (
                "statat(&self.parent,&self.name,AtFlags::SYMLINK_NOFOLLOW)",
                1,
            ),
            (
                "chownat(&claim.node.parent,&claim.node.name,None,Some(Gid::from_raw(reader_gid)),AtFlags::SYMLINK_NOFOLLOW)",
                1,
            ),
            (
                "chmodat(&claim.node.parent,&claim.node.name,Mode::from_raw_mode(0o660),AtFlags::empty())",
                1,
            ),
            (
                "unlinkat(&self.node.parent,&self.node.name,AtFlags::empty())",
                1,
            ),
            ("UnixListener::bind(path)", 1),
        ],
    )
}
pub(super) fn remainder(path: &Path, source: &str) -> String {
    if path.ends_with("crates/backup-worker/src/audit/endpoint.rs") {
        return checked_closed(
            source,
            &[
                "self.check(deadline)?;letsocket=connect(&self.path,self.producer,deadline)?;self.check(deadline)?;",
                "AuditSocket::new(UnixStream::from(fd),producer,deadline)",
            ],
            &[
                (
                    "net::socket_with(AddressFamily::UNIX,SocketType::STREAM,SocketFlags::NONBLOCK|SocketFlags::CLOEXEC,None)",
                    1,
                ),
                ("net::connect(&fd,&address)", 1),
            ],
        );
    }
    if !path.ends_with("crates/backup-worker/src/audit/endpoint/proof.rs") {
        return source.to_owned();
    }
    checked_closed(
        source,
        &[
            ".union(OFlags::NOFOLLOW)",
            "parts.len()<2||parts.len()>64",
            "Proof",
            "self.pins==other.pins",
            "stat.st_uid!=0&&stat.st_uid!=producer",
            "stat.st_mode&0o7022!=0",
            "parent_pin.uid!=producer||parent_pin.gid!=group||parent_pin.mode&0o7777!=0o750",
            "stat.st_gid!=group||stat.st_mode&0o7777!=0o660||stat.st_nlink!=1",
            "FileType::from_raw_mode(stat.st_mode)!=FileType::Socket",
            "proof._fds.push(fd)",
            "[\"system.posix_acl_access\",\"system.posix_acl_default\"]",
        ],
        &[
            ("fs::{self,FileType,Mode,OFlags,Stat}", 1),
            ("fs::open(\"/\",FLAGS,Mode::empty())", 1),
            (
                "fs::openat(parent,*name,iffinal_component{OFlags::PATH|OFlags::NOFOLLOW|OFlags::CLOEXEC}else{FLAGS},Mode::empty())",
                1,
            ),
            ("fs::fstat(fd)", 2),
            ("fs::fgetxattr(fd,name,&mutbyte[..])", 1),
            (
                "fs::getxattr(&pinned,\"system.posix_acl_access\",&mutbyte[..])",
                1,
            ),
        ],
    )
}
