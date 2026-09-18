//! 登记程序自身的秘密驻留纪律：唯一环境读取点、加固最先发生、不派生子进程。
//!
//! 加固实现本身（四项措施、固定分类、真实子进程内核回读）住在
//! `studiolo-host-config` 的 `process_safety`，测试也在那里；本文件只钉住
//! **本程序**与那份共享实现的接线方式，以及本程序自有缓冲的零化。
//!
//! 夹具里只有固定的非秘密标记，绝不含任何真实种子、验证码或口令。

use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

/// 固定的非秘密标记：只需要一段可检索的字节，不是秘密。
const MARKER_VALUE: &str = "fictional-residency-marker-not-a-secret";

// ---------------------------------------------------------------------------
// 自有缓冲的零化：只断言安全 Rust 能观察到的部分
// ---------------------------------------------------------------------------

#[test]
fn an_owned_secret_buffer_is_wiped_in_place_before_it_is_released() {
    // 环境槽位读出的是 Vec<u8>，种子解码与验证码输入是定长数组，两种都覆盖。
    let mut decoded = Zeroizing::new([7_u8; 20]);
    assert!(decoded.iter().any(|byte| *byte != 0));
    decoded.zeroize();
    assert!(decoded.iter().all(|byte| *byte == 0), "定长缓冲未就地清零");

    let mut received = Zeroizing::new(MARKER_VALUE.as_bytes().to_vec());
    assert!(!received.is_empty());
    received.zeroize();
    assert!(received.is_empty(), "变长缓冲未清零并截断");
}

#[test]
fn releasing_a_zeroizing_container_really_runs_the_wipe() {
    // 释放后的内存在安全 Rust 里读不到，改为证明「释放确实触发了零化」这一环。
    let wipes = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    drop(Zeroizing::new(WipeWitness(wipes.clone())));
    assert_eq!(wipes.get(), 1, "Zeroizing 释放时没有调用零化");
}

struct WipeWitness(std::rc::Rc<std::cell::Cell<usize>>);

impl Zeroize for WipeWitness {
    fn zeroize(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

// ---------------------------------------------------------------------------
// 源码守卫：把「只有一个读取点」「加固最先发生」从注释变成可执行约束
// ---------------------------------------------------------------------------

#[test]
fn the_secret_environment_slot_has_exactly_one_reader_in_the_whole_program() {
    let mut readers = Vec::new();
    for (path, source) in admin_sources() {
        let code = without_comments(&source);
        assert_eq!(
            code.matches("env::").count(),
            code.matches("std::env::").count(),
            "{} 用了简写的 env::，守卫无法逐点核对",
            file_name(&path)
        );
        let count = code.matches("std::env::").count();
        if count > 0 {
            readers.push((file_name(&path), count));
        }
    }
    readers.sort();
    assert_eq!(
        readers,
        vec![("environment.rs".to_owned(), 1), ("main.rs".to_owned(), 1)],
        "读取进程环境的地方必须只有 environment.rs 的种子槽位与 main.rs 的参数；\
         系统环境副本无法清除，每多一个读取点就多一份已零化秘密的新副本"
    );
}

#[test]
fn hardening_runs_before_any_argument_secret_or_configuration_is_read() {
    let code = without_comments(&read_admin_source("main.rs"));
    let hardened = code
        .find("process_safety::harden()")
        .expect("run() 必须先调用共享的 process_safety::harden()");
    let recorded = code
        .find("秘密驻留面：")
        .expect("harden() 的逐项分类必须被记录，不能只返回不记录");
    assert!(hardened < recorded);
    for later in [
        "std::env::args_os",
        "Config::load",
        "environment::read_seed",
    ] {
        let at = code
            .find(later)
            .unwrap_or_else(|| panic!("main.rs 应当包含 {later}"));
        assert!(recorded < at, "{later} 出现在加固与记录之前");
    }
}

/// 加固只能来自共享实现；本程序不得再自己写一份部分收紧。
///
/// 这一条钉住的正是「修一个漏一片」：三个持秘密程序各写一份、各缺几项，
/// 曾经让非因子模式的服务器完全没有收紧。
#[test]
fn the_registration_program_owns_no_private_copy_of_the_hardening() {
    for (path, source) in admin_sources() {
        let code = without_comments(&source);
        for syscall in [
            "setrlimit",
            "getrlimit",
            "Resource::Core",
            "set_dumpable_behavior",
            "mlockall",
            "remove_var",
        ] {
            assert!(
                !code.contains(syscall),
                "{} 自行调用了 {syscall}：收紧必须只经 studiolo_host_config::process_safety::harden()",
                file_name(&path)
            );
        }
    }
    let code = without_comments(&read_admin_source("main.rs"));
    assert!(
        code.contains("studiolo_host_config::process_safety::harden()"),
        "main.rs 必须调用共享实现，而不是本地同名函数"
    );
}

#[test]
fn the_registration_program_never_spawns_a_child_so_nothing_inherits_the_slot() {
    // 系统环境副本清不掉，只能保证它不被带进任何子进程。架构守卫已在工作区级
    // 禁止 Command::new；这里重述一遍，是因为加固的时序契约直接依赖它。
    for (path, source) in admin_sources() {
        let code = without_comments(&source);
        for pattern in ["Command::new(", "process::Command"] {
            assert!(
                !code.contains(pattern),
                "{} 出现了 {pattern}：派生子进程会把未清除的环境槽位带出去",
                file_name(&path)
            );
        }
    }
}

fn admin_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn admin_sources() -> Vec<(PathBuf, String)> {
    let mut entries = std::fs::read_dir(admin_directory())
        .expect("读取 auth-admin 源码目录")
        .map(|entry| entry.expect("读取源码目录条目").path())
        .filter(|path| path.extension().is_some_and(|value| value == "rs"))
        .collect::<Vec<_>>();
    entries.sort();
    assert!(entries.len() >= 7, "源码目录读取不完整，守卫会漏扫");
    entries
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(&path).expect("读取 auth-admin 源码");
            (path, source)
        })
        .collect()
}

fn read_admin_source(name: &str) -> String {
    std::fs::read_to_string(admin_directory().join(name)).expect("读取指定 auth-admin 源码")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("源码有文件名")
        .to_string_lossy()
        .into_owned()
}

/// 去掉整行注释：文档里会举例提到 `std::env::remove_var`，不能被守卫误判为调用。
fn without_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}
