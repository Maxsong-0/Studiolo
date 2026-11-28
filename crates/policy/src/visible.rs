//! 看不见的字符：连接器带进来的项目名、分支名里一个都不收（ADR-046）；MCP 握手里客户端的自报名去掉它们再记审计。
//!
//! 网页那边按 Unicode 属性拒（`web/src/presenceModel.ts`），整组读不到；这里少收一个就会出现「服务端收了、
//! 网页整组读不到」，所以网页用例逐码点读这张表核对「网页拒的，这里都拒」。
//!
//! 只管这两处。仓库里别处还有几份（`vault-broker` 与 `host-config` 的 `paths.rs`、`workbench` 的 `rust_paths.rs` 与
//! `operations_parse.rs`、网页 `cargoDependencies.ts` 与 `rustSourceValidation.ts`）：口径、用例各是各的，和这张互有出入
//! （比如前两份多到 U+13455，并且连控制字符一起拒），没有并过来。改这里不会改到那些，也不能直接拿这张换掉它们。

/// 格式字符（`Cf`）、行段分隔符（`Zl`、`Zp`）、默认可忽略字符（`Default_Ignorable_Code_Point`）和盲文空格，
/// 按码点升序的闭区间。
pub const INVISIBLE: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x206F),
    (0x2800, 0x2800),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
];

/// 这个字符看不见：能让界面上的名字读成别的样子，或夹带文字。
#[must_use]
pub fn invisible(character: char) -> bool {
    let code = u32::from(character);
    INVISIBLE
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code))
}
