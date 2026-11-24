use super::builder::{self, DIRECTORY, REGULAR, directory, end, file, header, pax, sha};
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
    let mut out = file("staging/sealed-中文 rootx/seal.json", b"{}");
    out.extend(stream(&good));
    cases.push(("根的同名兄弟", out));
    let mut out = file("other/seal.json", b"{}");
    out.extend(stream(&good));
    cases.push(("根外面的文件", out));
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
    let mut block = header(&named("seal.json"), REGULAR, 2);
    block[257..265].copy_from_slice(b"ustar  \0");
    builder::seal(&mut block);
    cases.push(("GNU 魔数", [block.to_vec(), stream(&good)].concat()));
    // 只有校验和字段不对：名字、长度、正文都对。
    let mut out = file(&named("seal.json"), b"{}");
    let at = out
        .chunks(512)
        .position(|block| block[156] == REGULAR && &block[257..263] == b"ustar\0")
        .unwrap()
        * 512;
    let block: &mut [u8; 512] = (&mut out[at..at + 512]).try_into().unwrap();
    builder::seal(block);
    let sum = u64::from_str_radix(std::str::from_utf8(&block[148..154]).unwrap(), 8).unwrap();
    block[148..156].copy_from_slice(format!("{:06o}\0 ", sum + 1).as_bytes());
    cases.push(("校验和不对", instead_of_seal(out)));
    let mut block = header(&named("seal.json"), REGULAR, 2);
    block[124] = 0x80;
    builder::seal(&mut block);
    cases.push(("base-256 长度", [block.to_vec(), stream(&good)].concat()));
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
    let mut out = pax(&[("path", "x")]);
    out.extend(pax(&[("path", "y")]));
    out.extend(stream(&good));
    cases.push(("两个 PAX 头连着", out));
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
    let mut block = header(&named("data"), DIRECTORY, 0);
    block[124..136].copy_from_slice(b"00000000001\0");
    builder::seal(&mut block);
    cases.push(("目录带长度", [block.to_vec(), stream(&good)].concat()));
    // 对照：上面只坏一处的那几种拼法，不坏的时候本身能核过。
    for good_entry in [
        file(&named("seal.json"), b"{}"),
        builder::entry(&named("seal.json"), REGULAR, b"{}"),
        custom(&[("path", seal.as_str()), ("size", "2")], b"{}"),
        custom(&[("path", seal.as_str())], b"{}"),
    ] {
        builder::check(&expected, &instead_of_seal(good_entry), 512).unwrap();
    }
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
    assert!(expected.file("a", 1, "ABC").is_err());
    assert!(expected.file("a", 1, &digest.to_uppercase()).is_err());
    expected.file("a", 1, &digest).unwrap();
    assert!(expected.file("a", 1, &digest).is_err());
    expected.directory("d").unwrap();
    assert!(expected.directory("d").is_err());
}
