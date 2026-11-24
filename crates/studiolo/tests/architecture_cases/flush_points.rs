//! 拷大文件写到一半要落盘（BUG-054）：没落盘的页在有内存上限的服务里回收不了，落不落盘从结果上看不出来，
//! 行为用例只数得到「该落的时候到了」，真的调没调 `sync_data` 靠这里守。
//!
//! 按记号比（`rust_lexer`）：注释掉的、写进字符串的都不算数；连着前后文一起比，挪进循环、夹一个 `#[cfg]` 也算没有。
use crate::support::{rust_lexer::lex, workspace_root};
use std::fs;

/// 哪个文件里哪几段要是活代码，各恰好一处。
const FLUSH_POINTS: [(&str, &[&str]); 4] = [
    (
        "crates/checkpoint-storage/src/export.rs",
        &["StepResult::More => flush.written(step_bytes, || target.flush())?,"],
    ),
    (
        "crates/checkpoint-storage/src/paths.rs",
        &["pub fn flush(&self) -> Result<(), Error> { self.file_pin.sync_data().map_err(io)?;"],
    ),
    (
        "crates/backup/src/checkpoint/staging/io/files.rs",
        &[
            "let file = &self.file; self.flush.written(bytes.len() as u64, || flush(file)) }",
            "fn flush(file: &File) -> Result<()> { \
             file.sync_data().map_err(|_| StagingError::Unavailable)?;",
        ],
    ),
    (
        "crates/vault-broker/src/checkpoint/capture.rs",
        &[
            "let mut flush = Flush::default(); loop {",
            "output.write_all(&self.buffer[..read]).map_err(|e| BrokerError::io(\"\", e))?; \
             flush.written(read as u64, || flush_halfway(&output))?; }",
            "fn flush_halfway(output: &File) -> Result<(), BrokerError> { \
             output.sync_data().map_err(|e| BrokerError::io(\"\", e))?;",
        ],
    ),
];

/// `anchors` 在源码里都恰好有一处活代码。
fn flushes(source: &str, anchors: &[&str]) -> bool {
    let tokens = lex(source);
    anchors.iter().all(|anchor| {
        let anchor = lex(anchor);
        tokens
            .windows(anchor.len())
            .filter(|window| *window == anchor)
            .count()
            == 1
    })
}

fn source(path: &str) -> String {
    fs::read_to_string(workspace_root().join(path)).unwrap()
}

#[test]
fn large_copies_flush_while_they_are_written() {
    for (path, anchors) in FLUSH_POINTS {
        let source = source(path);
        assert!(flushes(&source, anchors), "{path}");
        // 换行、空白、注释不影响判断。
        let reformatted = source.replace(";\n", "; // 说明\n\n");
        assert!(flushes(&reformatted, anchors), "{path}");
    }
}

/// 每一处改错都要判红：注释掉、数 0 个字节、吞掉失败、换成不落盘的调用、只在用例里落、计数挪进循环。
#[test]
fn every_way_of_losing_a_flush_is_caught() {
    const EXPORT: &str = "flush.written(step_bytes, || target.flush())?,";
    const PINNED: &str = "self.file_pin.sync_data().map_err(io)?;";
    const STAGED: &str = "self.flush.written(bytes.len() as u64, || flush(file))";
    const STAGED_SYNC: &str = "file.sync_data().map_err(|_| StagingError::Unavailable)?;";
    const COPIED: &str = "flush.written(read as u64, || flush_halfway(&output))?;";
    const COUNTED: &str = "let mut flush = Flush::default();\n        loop {";
    for (file, before, after) in [
        (
            0,
            EXPORT,
            "{} // flush.written(step_bytes, || target.flush())?,",
        ),
        (0, EXPORT, "flush.written(0, || target.flush())?,"),
        (
            0,
            EXPORT,
            "drop(flush.written(step_bytes, || target.flush())),",
        ),
        (1, PINNED, "// self.file_pin.sync_data().map_err(io)?;"),
        (
            1,
            PINNED,
            "#[cfg(test)]\n        self.file_pin.sync_data().map_err(io)?;",
        ),
        (1, PINNED, "let _ = self.file_pin.sync_data();"),
        (
            2,
            STAGED,
            "Ok(()) // self.flush.written(bytes.len() as u64, || flush(file))",
        ),
        (2, STAGED, "self.flush.written(0, || flush(file))"),
        (
            2,
            STAGED_SYNC,
            "// file.sync_data().map_err(|_| StagingError::Unavailable)?;",
        ),
        (2, STAGED_SYNC, "let _ = file.sync_data();"),
        (
            3,
            COPIED,
            "// flush.written(read as u64, || flush_halfway(&output))?;",
        ),
        (3, COPIED, "flush.written(0, || flush_halfway(&output))?;"),
        (
            3,
            COPIED,
            "let _ = flush.written(read as u64, || flush_halfway(&output));",
        ),
        (
            3,
            COPIED,
            "#[cfg(test)]\n            flush.written(read as u64, || flush_halfway(&output))?;",
        ),
        (
            3,
            COUNTED,
            "loop {\n            let mut flush = Flush::default();",
        ),
        (
            3,
            "output\n        .sync_data()",
            "output\n        .metadata()",
        ),
    ] {
        let (path, anchors) = FLUSH_POINTS[file];
        let source = source(path);
        assert_eq!(source.matches(before).count(), 1, "{path}：{before}");
        assert!(
            !flushes(&source.replacen(before, after, 1), anchors),
            "{path}：改成「{after}」没判红"
        );
    }
}
