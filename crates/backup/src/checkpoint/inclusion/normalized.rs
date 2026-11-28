//! 已规范化的绝对路径：「在不在某个根之内」只对这个类型有意义。
//!
//! `Path::starts_with` 只逐段比字面，`..` 会让它两头都错：`/a/b/../c` 看着在 `/a/b` 里（其实在
//! `/a`），`/a/c/x` 看着不在 `/a/b/../c` 里（其实就在那儿）。相对路径更要看进程当时在哪个目录。
//! 所以比较之前先在构造这一步拒掉没规范化的写法：不访问文件系统、不解析链接、不替调用方改写。

use std::path::{Path, PathBuf};

/// 路径哪里没规范化；只说类别，不带路径本身。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathDefect {
    /// 不是以 `/` 开头的绝对路径。
    Relative,
    /// 有空段：重复斜杠、尾斜杠，或根目录本身。
    EmptySegment,
    /// 有 `.` 段。
    CurrentDirectory,
    /// 有 `..` 段。
    ParentDirectory,
}

impl std::fmt::Display for PathDefect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Relative => "不是绝对路径",
            Self::EmptySegment => "有空段",
            Self::CurrentDirectory => "有 . 段",
            Self::ParentDirectory => "有 .. 段",
        })
    }
}

/// 规范化的绝对路径：以 `/` 开头，每一段都不空、不是 `.`、不是 `..`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedPath(PathBuf);

impl NormalizedPath {
    /// 只认规范化的写法；一处不合格就拒，不替调用方改写。
    pub fn parse(path: &Path) -> Result<Self, PathDefect> {
        // 不用 `Path::components()`：它把空段和中间的 `.` 悄悄吞掉，看不出原来有没有。
        let Some(segments) = path.as_os_str().as_encoded_bytes().strip_prefix(b"/") else {
            return Err(PathDefect::Relative);
        };
        for segment in segments.split(|byte| *byte == b'/') {
            match segment {
                b"" => return Err(PathDefect::EmptySegment),
                b"." => return Err(PathDefect::CurrentDirectory),
                b".." => return Err(PathDefect::ParentDirectory),
                _ => {}
            }
        }
        Ok(Self(path.to_owned()))
    }

    /// 已核过的路径。
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}
