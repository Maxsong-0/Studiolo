//! 正文搬运端口与固定帧落库流程；本模块自己不打开任何文件、不发起任何请求。

use super::{
    cas::{MirrorCas, ObjectId},
    error::ExecutorError,
    journal::{IssuedJob, JobOutcome},
};
use crate::{Action, FileVersion, Side};
use studiolo_core::VaultPath;

/// 搬运侧失败闭集；没有一个变体表示「可能已安装」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferFault {
    /// 来源或 sink 当前不可用。
    Unavailable,
    /// 冻结版本对应的来源对象已不存在。
    SourceMissing,
    /// 实际正文与冻结证据的摘要或字节数不符。
    ContentMismatch,
    /// 来源在声明字节数之前结束。
    Truncated,
    /// 受控镜像明确拒绝本次落库。
    Rejected,
}

/// 受控镜像侧的内容寻址落库出口；实现方负责真实字节，本核只给固定帧。
pub trait MirrorObjectSink {
    /// 开始接收一个新对象；同一时刻最多一个。
    fn begin(&mut self) -> Result<(), TransferFault>;
    /// 接收一帧不超过 64 KiB 的正文。
    fn accept(&mut self, frame: &[u8]) -> Result<(), TransferFault>;
    /// 按最终内容地址封口。
    fn finish(&mut self, id: ObjectId) -> Result<(), TransferFault>;
    /// 放弃当前未封口对象。
    fn discard(&mut self) -> Result<(), TransferFault>;
}

/// 冻结版本的正文来源；实现方须核对自己读到的确实是该版本。
pub trait MirrorContent {
    /// 打开指定侧、指定路径的指定版本；版本不符必须失败而不是给出别的字节。
    fn open(
        &mut self,
        side: Side,
        path: &VaultPath,
        version: &FileVersion,
    ) -> Result<(), TransferFault>;
    /// 读取下一帧，返回 0 表示结束；缓冲区长度即固定帧上限。
    fn next_frame(&mut self, buffer: &mut [u8]) -> Result<usize, TransferFault>;
}

/// 已 Issued 动作的实际执行者；无法证明终态时只能返回 [`JobOutcome::Unknown`]。
pub trait JobBackend {
    /// 按原幂等键执行一次；禁止用退出码、相同内容或超时伪造终态。
    fn apply(&mut self, job: &IssuedJob, action: &Action, staged: Option<ObjectId>) -> JobOutcome;
}

/// 按固定帧把一个冻结版本落进 MirrorCAS 并占用唯一 Prepared 槽。
pub(crate) fn stage(
    cas: &mut MirrorCas,
    content: &mut dyn MirrorContent,
    sink: &mut dyn MirrorObjectSink,
    side: Side,
    path: &VaultPath,
    version: &FileVersion,
) -> Result<ObjectId, ExecutorError> {
    content
        .open(side, path, version)
        .map_err(ExecutorError::Transfer)?;
    let expected = ObjectId::from_digest(version.sha256);
    cas.begin_receive(version.bytes, Some(expected), sink)
        .map_err(ExecutorError::Cas)?;
    match stream(cas, content, sink) {
        Ok(id) => {
            cas.prepare(id).map_err(ExecutorError::Cas)?;
            Ok(id)
        }
        Err(error) => {
            // 中途失败一律撤销未封口对象；不留下半截可被当成已保全的记录。
            if cas.is_receiving() {
                cas.abort_receive(sink).map_err(ExecutorError::Cas)?;
            }
            Err(error)
        }
    }
}

fn stream(
    cas: &mut MirrorCas,
    content: &mut dyn MirrorContent,
    sink: &mut dyn MirrorObjectSink,
) -> Result<ObjectId, ExecutorError> {
    let mut buffer = vec![0_u8; cas.limits().frame_bytes];
    loop {
        let read = content
            .next_frame(&mut buffer)
            .map_err(ExecutorError::Transfer)?;
        if read == 0 {
            break;
        }
        let frame = buffer
            .get(..read)
            .ok_or(ExecutorError::Transfer(TransferFault::ContentMismatch))?;
        cas.accept_frame(frame, sink).map_err(ExecutorError::Cas)?;
    }
    cas.finish_receive(sink).map_err(ExecutorError::Cas)
}
