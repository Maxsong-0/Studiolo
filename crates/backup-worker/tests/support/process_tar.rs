use super::builder::{self, REGULAR, directory, end, file, header, pax, sha};
use super::*;

const ROOT: &str = "/staging/sealed-中文 root";
const NOTE: &[u8] = b"# note\n";

/// 文件名 93 字节：加上 ASCII 的根放不进 100 字节的名字字段，要按 `/` 拆进前缀字段。
fn long() -> String {
    format!("data/{}.md", "a".repeat(90))
}

fn big() -> Vec<u8> {
    (0..70_000_u32).map(|n| (n % 251) as u8).collect()
}

/// 一份虚构封存：中文和空格的路径、空文件、跨好几块的大文件、要拆进 ustar 前缀的长名字。
fn files() -> Vec<(String, Vec<u8>)> {
    vec![
        ("seal.json".into(), b"{}".to_vec()),
        ("data/笔记 a;$.md".into(), NOTE.to_vec()),
        ("data/空".into(), Vec::new()),
        ("data/sub/big.bin".into(), big()),
        (long(), b"long".to_vec()),
    ]
}

fn expectation() -> TreeExpectation {
    let mut expected = TreeExpectation::new(Path::new(ROOT)).unwrap();
    for name in ["data", "data/sub"] {
        expected.directory(name).unwrap();
    }
    for (path, body) in files() {
        expected
            .file(&path, body.len() as u64, &sha(&body))
            .unwrap();
    }
    expected
}

fn named(relative: &str) -> String {
    format!("{}/{relative}", &ROOT[1..])
}

/// restic 写出来的样子：子目录先于它里面的文件，封存根自己不写。
fn stream(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = directory(&named("data"));
    out.extend(directory(&named("data/sub")));
    for (path, body) in files {
        out.extend(file(&named(path), body));
    }
    out.extend(end());
    out
}

/// 自己拼的一项：一个带给定记录的 PAX 头、一个占位名字的 ustar 普通文件头和正文。
fn custom(records: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = pax(records);
    out.extend(header("placeholder", REGULAR, body.len() as u64));
    out.extend(body);
    out.resize(out.len().div_ceil(512) * 512, 0);
    out
}

/// 把 `seal.json` 换成自己拼的一项，其余照 restic 的写法：坏样本只坏这一处，别的检查拦不住它。
fn instead_of_seal(entry: Vec<u8>) -> Vec<u8> {
    [entry, stream(&files()[1..])].concat()
}

/// 把 `data` 目录项换成自己拼的一项，其余照 restic 的写法（同上，坏样本只坏这一处）。
fn instead_of_data_directory(entry: Vec<u8>) -> Vec<u8> {
    let mut out = entry;
    out.extend(directory(&named("data/sub")));
    for (path, body) in files() {
        out.extend(file(&named(&path), &body));
    }
    out.extend(end());
    out
}

/// 一个 PAX 正文原样给的项（可以故意写坏），后面跟占位名字的 ustar 头和 `seal.json` 的正文 `{}`。
fn seal_with_pax_body(body: &[u8]) -> Vec<u8> {
    let mut out = builder::pax_body(body);
    out.extend(header("placeholder", REGULAR, 2));
    out.extend(b"{}");
    out.resize(out.len().div_ceil(512) * 512, 0);
    out
}

/// `seal.json` 的 PAX path 记录。
fn seal_path() -> Vec<u8> {
    builder::record(b"path", named("seal.json").as_bytes())
}

#[test]
fn the_stream_restic_writes_passes_however_it_is_split_and_the_root_entry_is_optional() {
    let expected = expectation();
    let good = stream(&files());
    for chunk in [1, 7, 511, 512, 513, 4096, good.len()] {
        builder::check(&expected, &good, chunk).unwrap_or_else(|_| panic!("chunk {chunk}"));
    }
    let mut with_root = directory(&ROOT[1..]);
    with_root.extend(&good);
    builder::check(&expected, &with_root, 4096).unwrap();
    // 结尾之后跟的全零补齐不碍事。
    let mut padded = good.clone();
    padded.extend(vec![0; 9 * 1024]);
    builder::check(&expected, &padded, 4096).unwrap();
}

#[test]
fn every_wrong_shape_of_the_stream_fails_the_read_back() {
    let expected = expectation();
    let good = files();
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut changed = good.clone();
    changed.remove(1);
    cases.push(("少一个文件", stream(&changed)));
    let mut changed = good.clone();
    changed.push(("data/extra".into(), b"x".to_vec()));
    cases.push(("多一个文件", stream(&changed)));
    let mut changed = good.clone();
    changed.push(good[1].clone());
    cases.push(("同一个文件两遍", stream(&changed)));
    let mut changed = good.clone();
    changed[1].1 = b"# NOTE\n".to_vec();
    cases.push(("内容不同、长度相同", stream(&changed)));
    let mut changed = good.clone();
    changed[3].1.pop();
    cases.push(("少一个字节", stream(&changed)));
    let mut out = stream(&good);
    out.truncate(out.len() - 1024);
    cases.push(("没有结尾", out));
    let mut out = stream(&good);
    out.truncate(out.len() - 512);
    cases.push(("只有一个全零块", out));
    let mut out = stream(&good);
    out.truncate(out.len() - 1024 - 600);
    cases.push(("断在半路", out));
    let mut out = stream(&good);
    out.extend(file(&named("data/after"), b"x"));
    cases.push(("结尾之后又来一项", out));
    let mut out = stream(&good);
    out.push(1);
    cases.push(("结尾之后不是零", out));
    let mut out = directory(&named("data"));
    out.extend(stream(&good));
    cases.push(("目录两遍", out));
    let mut out = directory(&named("data"));
    for (path, body) in &good {
        out.extend(file(&named(path), body));
    }
    out.extend(end());
    cases.push(("少一个目录", out));
    let mut out = directory(&named("data/unknown"));
    out.extend(stream(&good));
    cases.push(("清单外的目录", out));
    let mut out = directory(&named("data"));
    out.extend(directory(&named("data/unknown")));
    for (path, body) in &good {
        out.extend(file(&named(path), body));
    }
    out.extend(end());
    cases.push(("清单外的目录顶替清单里的一个", out));
    // 根的同名兄弟、根外面：去掉根之后剩下的正好是清单里的名字，只有「在根下」这一条拦得住。
    let sibling = format!("{}seal.json", &ROOT[1..]);
    cases.push(("根的同名兄弟", instead_of_seal(file(&sibling, b"{}"))));
    let outside = format!("other/{}", named("seal.json"));
    cases.push(("根外面的文件", instead_of_seal(file(&outside, b"{}"))));
    let sibling = format!("{}data", &ROOT[1..]);
    cases.push((
        "根的同名兄弟目录",
        instead_of_data_directory(directory(&sibling)),
    ));
    let outside = format!("other/{}", named("data"));
    cases.push((
        "根外面的目录",
        instead_of_data_directory(directory(&outside)),
    ));
    // 名字、长度、正文都和清单对得上，只是类型不对。
    for (name, kind) in [
        ("硬链接", b'1'),
        ("符号链接", b'2'),
        ("字符设备", b'3'),
        ("命名管道", b'6'),
        ("连续文件", b'7'),
        ("全局 PAX", b'g'),
    ] {
        cases.push((
            name,
            instead_of_seal(builder::entry(&named("seal.json"), kind, b"{}")),
        ));
    }
    // ustar 头只坏一处：名字、长度、正文都对。
    let seal_file = || file(&named("seal.json"), b"{}");
    cases.push((
        "GNU 魔数",
        instead_of_seal(builder::retouch(seal_file(), |block| {
            block[257..265].copy_from_slice(b"ustar  \0")
        })),
    ));
    let mut out = builder::retouch(seal_file(), |_| {});
    let at = out
        .chunks(512)
        .position(|block| block[156] == REGULAR && &block[257..263] == b"ustar\0")
        .unwrap()
        * 512;
    let sum =
        u64::from_str_radix(std::str::from_utf8(&out[at + 148..at + 154]).unwrap(), 8).unwrap();
    out[at + 148..at + 156].copy_from_slice(format!("{:06o}\0 ", sum + 1).as_bytes());
    cases.push(("校验和不对", instead_of_seal(out)));
    // 长度 2 写成 base-256（首字节高位置 1）：restic 要写这么大的数时会改用 PAX size。
    cases.push((
        "base-256 长度",
        instead_of_seal(builder::retouch(seal_file(), |block| {
            block[124..136].fill(0);
            block[124] = 0x80;
            block[135] = 2;
        })),
    ));
    let mut out = file(&named("seal.json"), b"{}");
    let at = out.len() - 1;
    out[at] = 7;
    cases.push(("补齐不是零", [out, stream(&good[1..])].concat()));
    // PAX 记录只坏一处：路径、长度、正文都对得上清单。
    let seal = named("seal.json");
    for (name, extra) in [
        ("PAX 里 path 两遍", ("path", seal.as_str())),
        ("PAX 里 size 两遍", ("size", "2")),
        ("稀疏文件", ("GNU.sparse.map", "0,1")),
        ("链接名", ("linkpath", "data/空")),
    ] {
        let records = [("path", seal.as_str()), ("size", "2"), extra];
        cases.push((name, instead_of_seal(custom(&records, b"{}"))));
    }
    cases.push((
        "没写成 PAX 的稀疏头",
        instead_of_seal(custom(
            &[("path", seal.as_str()), ("GNU.sparse.size", "2")],
            b"{}",
        )),
    ));
    // PAX 记录本身写坏了：每样只坏一条，path 记录照常在前面。
    for (name, bad) in [
        ("PAX 记录长度比实际长", b"21 mtime=1700000000\n".to_vec()),
        ("PAX 记录长度比实际短", b"19 mtime=1700000000\n".to_vec()),
        ("PAX 记录缺换行", b"20 mtime=1700000000x".to_vec()),
        ("PAX 记录缺等号", b"20 mtime 1700000000\n".to_vec()),
        ("PAX 记录键为空", b"15 =1700000000\n".to_vec()),
        ("PAX 记录长度不是数字", b"2x mtime=1700000000\n".to_vec()),
        ("PAX size 带正号", builder::record(b"size", b"+2")),
        ("PAX size 不是数字", builder::record(b"size", b"2a")),
        ("PAX size 是空的", builder::record(b"size", b"")),
    ] {
        cases.push((
            name,
            instead_of_seal(seal_with_pax_body(&[seal_path(), bad].concat())),
        ));
    }
    cases.push((
        "PAX path 不是 UTF-8",
        instead_of_seal(seal_with_pax_body(&builder::record(b"path", b"\xff"))),
    ));
    cases.push((
        "PAX 正文超过 64 KiB",
        instead_of_seal(seal_with_pax_body(&pax_of_length(64 * 1024 + 1))),
    ));
    cases.push((
        "两个 PAX 头连着",
        instead_of_seal(
            [
                pax(&[("path", "x")]),
                custom(&[("path", seal.as_str())], b"{}"),
            ]
            .concat(),
        ),
    ));
    let mut out = stream(&good);
    let tail = out.split_off(out.len() - 1024);
    out.extend(pax(&[("path", "x")]));
    out.extend(tail);
    cases.push(("PAX 头后面直接结尾", out));
    let mut out = pax(&[("path", &named("seal.json")), ("size", "3")]);
    out.extend(header("x", REGULAR, 2));
    out.extend(b"{}");
    out.resize(out.len().div_ceil(512) * 512, 0);
    cases.push(("PAX 长度和清单不同", [out, stream(&good[1..])].concat()));
    cases.push((
        "目录带长度",
        instead_of_data_directory(builder::retouch(directory(&named("data")), |block| {
            block[124..136].copy_from_slice(b"00000000001\0")
        })),
    ));
    // 对照：上面只坏一处的那几种拼法，不坏的时候本身能核过。
    for good_entry in [
        file(&named("seal.json"), b"{}"),
        builder::retouch(seal_file(), |_| {}),
        builder::entry(&named("seal.json"), REGULAR, b"{}"),
        custom(&[("path", seal.as_str()), ("size", "2")], b"{}"),
        custom(&[("path", seal.as_str())], b"{}"),
        seal_with_pax_body(&seal_path()),
        seal_with_pax_body(&[seal_path(), builder::record(b"size", b"2")].concat()),
        seal_with_pax_body(&[seal_path(), b"20 mtime=1700000000\n".to_vec()].concat()),
        seal_with_pax_body(&pax_of_length(64 * 1024)),
    ] {
        builder::check(&expected, &instead_of_seal(good_entry), 512).unwrap();
    }
    builder::check(
        &expected,
        &instead_of_data_directory(directory(&named("data"))),
        512,
    )
    .unwrap();
    for (name, bytes) in cases {
        for chunk in [1, 512, 1 << 20] {
            assert_eq!(
                builder::check(&expected, &bytes, chunk),
                Err(ProcessError::Protocol),
                "{name}（每段 {chunk} 字节）"
            );
        }
    }
}

#[test]
fn a_size_from_a_pax_record_and_a_split_ustar_name_are_both_read() {
    let mut expected = TreeExpectation::new(Path::new(ROOT)).unwrap();
    expected.file("seal.json", 2, &sha(b"{}")).unwrap();
    let mut out = pax(&[("path", &named("seal.json")), ("size", "2")]);
    out.extend(header("placeholder", REGULAR, 0));
    out.extend(b"{}");
    out.resize(out.len().div_ceil(512) * 512, 0);
    out.extend(end());
    builder::check(&expected, &out, 3).unwrap();
    // 拆进前缀字段的名字：前缀「…/data」，名字是一百来个字节的文件名。
    let ascii = "/staging/sealed-root";
    let mut expected = TreeExpectation::new(Path::new(ascii)).unwrap();
    expected.directory("data").unwrap();
    expected.file(&long(), 4, &sha(b"long")).unwrap();
    let name = format!("{}/{}", &ascii[1..], long());
    let block = header(&name, REGULAR, 4);
    assert_ne!(block[345], 0, "这个名字要用到前缀字段");
    let mut out = directory(&format!("{}/data", &ascii[1..]));
    out.extend(file(&name, b"long"));
    out.extend(end());
    builder::check(&expected, &out, 512).unwrap();
}

/// 正好 `total` 字节的 PAX 正文：`seal.json` 的 path 记录，再用一条 `comment` 记录补足。
fn pax_of_length(total: usize) -> Vec<u8> {
    let filler = total - seal_path().len();
    // 「长度 comment=…\n」：这里长度是 5 位数，所以值是 filler - 15 个字节。
    let record = builder::record(b"comment", &vec![b'a'; filler - 15]);
    assert_eq!(record.len(), filler);
    [seal_path(), record].concat()
}

/// restic 除了 path、size 还会写的记录（时间、属主、扩展属性）都不看值：扩展属性的值是原样字节，
/// 可以不是 UTF-8、可以带 NUL（Go 的 archive/tar 只禁这些键里有 NUL）。
#[test]
fn the_other_pax_records_restic_writes_are_ignored_whatever_their_values() {
    let expected = expectation();
    let body = [
        seal_path(),
        builder::record(b"mtime", b"1700000000.123456789"),
        builder::record(b"atime", b"1700000001"),
        builder::record(b"ctime", b"1700000002"),
        builder::record(b"uname", "主人".as_bytes()),
        builder::record(b"gname", b"studiolo"),
        builder::record(b"SCHILY.xattr.user.blob", b"\xff\x00\x80 binary"),
        builder::record(
            b"SCHILY.xattr.system.posix_acl_access",
            &[2, 0, 0, 0, 1, 0, 6, 0, 0xff, 0xff, 0xff, 0xff],
        ),
        builder::record(b"SCHILY.acl.access", b"user::rw-,group::r--,other::---"),
    ]
    .concat();
    builder::check(&expected, &instead_of_seal(seal_with_pax_body(&body)), 512).unwrap();
    // 老式普通文件的类型字节是 NUL。
    let seal = named("seal.json");
    builder::check(
        &expected,
        &instead_of_seal(builder::entry(&seal, 0, b"{}")),
        512,
    )
    .unwrap();
    // PAX 头可以是空的：名字照 ustar 头的。
    let mut out = builder::pax_body(b"");
    out.extend(header(&seal, REGULAR, 2));
    out.extend(b"{}");
    out.resize(out.len().div_ceil(512) * 512, 0);
    builder::check(&expected, &instead_of_seal(out), 512).unwrap();
}

/// 空文件也核摘要：清单里记的摘要不是空内容的，读回就不算通过。
#[test]
fn an_empty_file_is_checked_against_its_recorded_digest_too() {
    let mut stream = file(&named("data/空"), b"");
    stream.extend(end());
    let mut expected = TreeExpectation::new(Path::new(ROOT)).unwrap();
    expected.file("data/空", 0, &sha(b"")).unwrap();
    builder::check(&expected, &stream, 512).unwrap();
    let mut expected = TreeExpectation::new(Path::new(ROOT)).unwrap();
    expected.file("data/空", 0, &sha(b"x")).unwrap();
    assert_eq!(
        builder::check(&expected, &stream, 512),
        Err(ProcessError::Protocol)
    );
}

#[test]
fn the_output_limit_covers_the_stream_and_stays_close_to_the_data() {
    let expected = expectation();
    let good = stream(&files());
    let data: u64 = files().iter().map(|(_, body)| body.len() as u64).sum();
    let mut sink = TarCheck::new(&expected);
    assert!(good.len() as u64 <= sink.limit());
    // 每项最多一个 PAX 头和它的正文、一个 ustar 头：7 项、每项不到 66 KiB，加结尾。
    assert!(sink.limit() < data + 7 * 66 * 1024 + 16 * 1024);
    sink.append(&good);
    sink.finish().unwrap();
}

#[test]
fn only_absolute_roots_relative_paths_and_full_digests_are_expected() {
    for root in ["relative", "/", "/a/../b", "/a//b", "/a/"] {
        assert!(TreeExpectation::new(Path::new(root)).is_err(), "{root}");
    }
    let mut expected = TreeExpectation::new(Path::new(ROOT)).unwrap();
    let digest = sha(b"x");
    for path in ["", "/abs", "a/../b", "a//b", "a/", "."] {
        assert!(expected.file(path, 1, &digest).is_err(), "{path}");
        assert!(expected.directory(path).is_err(), "{path}");
    }
    assert!(expected.file("big", FILE_LIMIT + 1, &digest).is_err());
    expected.file("big", FILE_LIMIT, &digest).unwrap();
    assert!(expected.file("a", 1, "ABC").is_err());
    assert!(expected.file("a", 1, &digest.to_uppercase()).is_err());
    expected.file("a", 1, &digest).unwrap();
    assert!(expected.file("a", 1, &digest).is_err());
    expected.directory("d").unwrap();
    assert!(expected.directory("d").is_err());
}
