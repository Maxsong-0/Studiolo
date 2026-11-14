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
        ]
    } else if path.ends_with("crates/local-server/src/backup/admission.rs") {
        &[
            "sealed:&SealedCheckpoint",
            "record.check_live()?;",
            "verify_source(&ledger,&record)?;",
            "letreceipt=ledger.append(event.clone())?;",
            "admission.verify()?;",
            "intent.event()!=&expected||intent.offset()>record.audit_offset",
            "tail.offset()!=record.audit_offset||tail.record_hash().to_hex()!=record.audit_hash",
            "receipt.event()!=expected||receipt.offset()<=cutoff",
        ]
    } else if path.ends_with("crates/local-server/src/backup/admission/record.rs") {
        &[
            "#[serde(deny_unknown_fields)]",
            "self.key.checkpoint_id!=self.source.checkpoint_id",
            "self.audit_offset==0",
            "verify_receipt(receipt,&record.event()?,record.audit_offset)?;",
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
