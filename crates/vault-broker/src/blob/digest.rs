//! 只允许固定大小缓冲，不能把正文经 read_to_end 或序列化中转到内存。
use super::{BlobIdentity, CHUNK_BYTES, OBJECT_LIMIT};
use crate::{BrokerError, Revision, VaultPath, VaultUuid};
use cap_std::fs::File;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use studiolo_core::RevisionHasher;
mod text;

pub(super) struct Digests {
    pub identity: BlobIdentity,
    pub revision: Option<Revision>,
}
pub(super) fn transfer(
    source: &mut File,
    destination: Option<&mut File>,
    limit: u64,
    binding: Option<(VaultUuid, &VaultPath)>,
) -> Result<Digests, BrokerError> {
    run(source, destination, limit, binding, false).map(|(digest, _)| digest)
}

pub(super) fn facts(
    source: &mut File,
    limit: u64,
    binding: Option<(VaultUuid, &VaultPath)>,
) -> Result<(Digests, bool), BrokerError> {
    run(source, None, limit, binding, true)
}

fn run(
    source: &mut File,
    mut destination: Option<&mut File>,
    limit: u64,
    binding: Option<(VaultUuid, &VaultPath)>,
    check_text: bool,
) -> Result<(Digests, bool), BrokerError> {
    if limit > OBJECT_LIMIT {
        return Err(BrokerError::WriterLimitExceeded);
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|e| BrokerError::io("rewind blob source", e))?;
    let mut body = Sha256::new();
    let mut revision = binding.map(|(vault, path)| RevisionHasher::new(vault, path));
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; CHUNK_BYTES];
    let mut text = text::Utf8::default();
    loop {
        // 观察已固定源长度，零额外探测字节；旧 transfer 仍保留原有超限探测语义。
        if check_text && bytes == limit {
            break;
        }
        let remaining = if check_text {
            limit - bytes
        } else {
            (limit - bytes).saturating_add(1)
        };
        let next = usize::try_from(remaining.min(CHUNK_BYTES as u64))
            .map_err(|_| BrokerError::WriterLimitExceeded)?;
        let size = source
            .read(&mut buffer[..next])
            .map_err(|e| BrokerError::io("read blob chunk", e))?;
        if size == 0 {
            break;
        }
        bytes = bytes
            .checked_add(size as u64)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if bytes > limit {
            return Err(BrokerError::WriterLimitExceeded);
        }
        body.update(&buffer[..size]);
        if check_text {
            text.update(&buffer[..size]);
        }
        #[cfg(test)]
        if check_text {
            crate::sync_observation::tests::hook(3);
        }
        if let Some(hasher) = &mut revision {
            hasher.update(&buffer[..size]);
        }
        if let Some(output) = &mut destination {
            output
                .write_all(&buffer[..size])
                .map_err(|e| BrokerError::io("write blob chunk", e))?;
        }
    }
    Ok((
        Digests {
            identity: BlobIdentity {
                bytes,
                sha256: crate::hex(&body.finalize()),
            },
            revision: revision.map(RevisionHasher::finish),
        },
        text.complete(),
    ))
}
