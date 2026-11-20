//! 人侧发行激活时，`deploy/human/release/snapshot_worker.py` 只许三个库目录里出现哪些名字；直接从它的源码读。
//!
//! 白名单的值（库文件名、SQLite 旁文件后缀、审计库另认的准入锁）跟着 Python 走，不在这里另抄一份；
//! 核对的写法（白名单由哪个函数给、三个库是不是都过一遍、准入锁只认空文件）按原样钉住：
//! Python 那边改了写法，这里先红，改的人回来核对这条测试还管不管用（vault BUG-040）。
use std::{collections::BTreeSet, path::Path};

/// 相对仓库根的路径。
const SOURCE: &str = "deploy/human/release/snapshot_worker.py";

/// 原样钉住的核对语句：库目录里的名字是 `expected_names` 的子集；三个库都经 `source` 核对；准入锁只认空文件。
const PINNED: [&str; 3] = [
    "    require({p.name for p in path.parent.iterdir()} <= expected_names(path))",
    "    identities = [source(path) for path, _ in DATABASES]",
    "        require(p.name != AUDIT_GATE or s.st_size == 0)",
];

/// 从 `snapshot_worker.py` 读出来的库目录白名单。
pub(super) struct Allowlist {
    /// `DATABASES` 里各库的文件名，照 Python 里的顺序。
    pub(super) databases: Vec<String>,
    /// 库文件名后面允许接的后缀（含空串，即库文件本身）。
    suffixes: Vec<String>,
    /// 审计库目录另认的准入锁文件名。
    gate: String,
    /// 认准入锁的那个库的文件名。
    gate_database: String,
}

impl Allowlist {
    /// 读仓库里的 `snapshot_worker.py`；写法对不上就 panic，说明哪一处变了。
    pub(super) fn load() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(SOURCE);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读不到 {SOURCE}：{error}"));
        Self::parse(&source)
    }

    fn parse(source: &str) -> Self {
        for pinned in PINNED {
            let found = source.lines().filter(|line| *line == pinned).count();
            assert_eq!(
                found, 1,
                "{SOURCE} 的核对写法变了，找不到（或不止一处）：{pinned}"
            );
        }
        let gate = quoted(between(source, "AUDIT_GATE = ", ""));
        let suffixes = between(
            source,
            "    names = {path.name + suffix for suffix in (",
            ")}",
        )
        .split(", ")
        .map(quoted)
        .collect();
        let gate_database = quoted(between(
            source,
            "    return (names | {AUDIT_GATE}) if path.name == ",
            " else names",
        ));
        let databases = databases(source);
        assert!(
            databases.contains(&gate_database),
            "{SOURCE} 里认准入锁的库 {gate_database} 不在 DATABASES 里"
        );
        Self {
            databases,
            suffixes,
            gate,
            gate_database,
        }
    }

    /// 这个库所在目录里允许出现的全部名字。
    pub(super) fn names(&self, database: &str) -> BTreeSet<String> {
        let mut names: BTreeSet<String> = self
            .suffixes
            .iter()
            .map(|suffix| format!("{database}{suffix}"))
            .collect();
        if database == self.gate_database {
            names.insert(self.gate.clone());
        }
        names
    }

    /// 准入锁的文件名：出现时必须是空文件。
    pub(super) fn gate(&self) -> &str {
        &self.gate
    }
}

/// 唯一一行以 `prefix` 开头、以 `suffix` 结尾的语句，去掉两头后剩下的部分。
fn between<'a>(source: &'a str, prefix: &str, suffix: &str) -> &'a str {
    let found: Vec<&str> = source
        .lines()
        .filter_map(|line| line.strip_prefix(prefix)?.strip_suffix(suffix))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "{SOURCE} 的写法变了，找不到（或不止一处）以「{prefix}」开头、以「{suffix}」结尾的一行"
    );
    found[0]
}

/// 单引号括起、里面没有引号和转义的 Python 字符串字面量。
fn quoted(literal: &str) -> String {
    let inner = literal
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .filter(|inner| !inner.contains(['\'', '"', '\\']))
        .unwrap_or_else(|| panic!("{SOURCE} 里 {literal} 不是这里认得的字符串写法"));
    inner.to_owned()
}

/// `DATABASES = (...)` 里每个 `ROOT / '目录/库文件'` 的库文件名。
fn databases(source: &str) -> Vec<String> {
    let block: Vec<&str> = source
        .lines()
        .skip_while(|line| !line.starts_with("DATABASES = ("))
        .scan(false, |closed, line| {
            // 收尾的那一行也要：它以 `))` 结束。
            let take = !*closed;
            *closed = line.ends_with("))");
            take.then_some(line)
        })
        .collect();
    let files: Vec<String> = block
        .iter()
        .map(|line| {
            let (_, rest) = line
                .split_once("(ROOT / '")
                .unwrap_or_else(|| panic!("{SOURCE} 的 DATABASES 写法变了：{line}"));
            let (relative, _) = rest.split_once('\'').unwrap();
            let (_, file) = relative
                .rsplit_once('/')
                .unwrap_or_else(|| panic!("{SOURCE} 的 DATABASES 写法变了：{line}"));
            file.to_owned()
        })
        .collect();
    assert!(!files.is_empty(), "{SOURCE} 里找不到 DATABASES");
    files
}
