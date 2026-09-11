//! 显式绑定根下的直属目录枚举；仅访问目录与元信息，不读取普通文件内容。

use std::collections::BTreeSet;

use cap_std::fs::{Dir, Metadata};
use studiolo_core::VaultPath;

use super::{ReadOnlyVault, limit, metadata, private, rooted, unsafe_node, visible};
use crate::BrokerError;

const MAX_ENTRIES: usize = 8192;
const MAX_PATH_BYTES: usize = 16 * 1024 * 1024;

impl ReadOnlyVault {
    /// 列出根或指定目录下的直属可见目录，按完整根相对路径排序；空目录返回空数组。
    ///
    /// None 表示已固定的根；路径中的隐藏分量仍被拒绝。所有枚举项（包括文件和
    /// 隐藏项）计入直属枚举预算，max_entries 必须为 1..=8192；可见路径合计最多
    /// 16 MiB。父路径安全名称检查沿用读取内核的每层 50,000 项上限。
    /// 普通文件仅验证元信息；链接、特殊类型、名称冲突、并发替换或超限均使整次
    /// 调用失败，不返回部分目录。私有模式继续拒绝隐藏项与不安全权限。
    /// Linux 使用内核 beneath/no-link/no-xdev；其他 Unix 沿固定父句柄 NOFOLLOW，
    /// 并复核设备和目录身份。根被改名后仍访问原句柄，不重新绑定宿主路径。
    /// 这些检查不构成文件系统事务快照；不支持的平台继续关闭访问。
    pub fn list_directories(
        &self,
        path: Option<&VaultPath>,
        max_entries: usize,
    ) -> Result<Vec<VaultPath>, BrokerError> {
        if max_entries == 0 || max_entries > MAX_ENTRIES {
            return Err(limit(
                "directory listing entry limit must be within 1..=8192",
            ));
        }
        let directory = self.listing_directory(path)?;
        private::directory(&self.root, self.private)?;
        private::directory(&directory, self.private)?;
        let before = directory_metadata(&directory)?;
        let candidates = self.directory_candidates(&directory, path, max_entries)?;
        let mut result = Vec::with_capacity(candidates.len());
        for (name, child_path, opened) in candidates {
            let child = rooted::scanned_directory(
                &self.root,
                &directory,
                &name,
                &child_path,
                self.private,
            )?;
            if !rooted::unchanged(&opened, &directory_metadata(&child)?) {
                return Err(BrokerError::UnstableRead(child_path));
            }
            result.push(child_path);
        }
        if !rooted::unchanged(&before, &directory_metadata(&directory)?) {
            return Err(limit("source directory changed during listing"));
        }
        // 重新从固定根解析选定路径，不能仅信任已经被移出原位置的父句柄。
        let current = self.listing_directory(path)?;
        if !rooted::unchanged(&before, &directory_metadata(&current)?) {
            return Err(limit("selected directory changed during listing"));
        }
        private::directory(&self.root, self.private)?;
        private::directory(&directory, self.private)?;
        result.sort();
        Ok(result)
    }

    fn listing_directory(&self, path: Option<&VaultPath>) -> Result<Dir, BrokerError> {
        match path {
            Some(path) => {
                visible(path)?;
                rooted::open_directory(&self.root, path, self.private)
            }
            None => self
                .root
                .try_clone()
                .map_err(|error| BrokerError::io("retain directory listing root", error)),
        }
    }

    fn directory_candidates(
        &self,
        directory: &Dir,
        prefix: Option<&VaultPath>,
        max_entries: usize,
    ) -> Result<Vec<(String, VaultPath, Metadata)>, BrokerError> {
        let mut candidates = Vec::new();
        let mut names = BTreeSet::new();
        let mut path_bytes = 0_usize;
        for (index, entry) in directory
            .entries()
            .map_err(|error| BrokerError::io("enumerate repository directories", error))?
            .enumerate()
        {
            if index >= max_entries {
                return Err(limit("directory listing entry limit exceeded"));
            }
            let entry = entry.map_err(|error| BrokerError::io("read directory entry", error))?;
            let name = entry.file_name();
            if name.as_encoded_bytes().starts_with(b".") {
                if self.private {
                    return Err(limit("private directory listing rejects hidden entries"));
                }
                continue;
            }
            let prefix = prefix.map_or("", VaultPath::as_str);
            path_bytes = path_bytes
                .checked_add(prefix.len())
                .and_then(|bytes| bytes.checked_add(usize::from(!prefix.is_empty())))
                .and_then(|bytes| bytes.checked_add(name.as_encoded_bytes().len()))
                .filter(|bytes| *bytes <= MAX_PATH_BYTES)
                .ok_or_else(|| limit("directory listing logical path bytes limit exceeded"))?;
            let name = name
                .to_str()
                .ok_or_else(|| limit("directory listing contains a non-UTF-8 name"))?;
            let raw = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let path = VaultPath::parse(raw)
                .map_err(|_| limit("directory listing contains a noncanonical name"))?;
            if !names.insert(name.to_lowercase()) {
                return Err(unsafe_node(&path, "case-colliding filesystem name"));
            }
            let kind = entry
                .file_type()
                .map_err(|error| BrokerError::io("inspect directory entry type", error))?;
            if kind.is_dir() {
                let child =
                    rooted::scanned_directory(&self.root, directory, name, &path, self.private)?;
                candidates.push((name.to_owned(), path, directory_metadata(&child)?));
            } else if kind.is_file() {
                // 不打开内容句柄；包括 Markdown 和源码在内的普通文件都只取元信息。
                metadata::stable_file(&self.root, directory, name, &path, self.private)?;
            } else {
                return Err(unsafe_node(
                    &path,
                    "directory listing rejects links and special entries",
                ));
            }
        }
        Ok(candidates)
    }
}

fn directory_metadata(directory: &Dir) -> Result<Metadata, BrokerError> {
    directory
        .dir_metadata()
        .map_err(|error| BrokerError::io("stat listed directory", error))
}
