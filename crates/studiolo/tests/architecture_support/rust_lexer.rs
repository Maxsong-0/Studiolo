//! 按源码文本做判定的守卫共用的 Rust 记号化：只切出标识符与标点，
//! 空白、注释、字符串与字符字面量一概不产生记号，守卫因此不会被它们误导或绕开。

pub(crate) fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// 切成标识符与标点记号：跳过空白、注释（块注释可嵌套）、字符串与字符字面量，
/// `::` 与 `!=` 各合成一个记号，原始标识符 `r#名字` 记作 `名字`。
pub(crate) fn lex(source: &str) -> Vec<String> {
    let chars = source.chars().collect::<Vec<_>>();
    let (mut tokens, mut index) = (Vec::new(), 0);
    while index < chars.len() {
        let rest = &chars[index..];
        index += match rest {
            [c, ..] if c.is_whitespace() => 1,
            ['/', '/', ..] => rest.iter().take_while(|&&c| c != '\n').count(),
            ['/', '*', ..] => block_comment_len(rest),
            ['"', tail @ ..] => 1 + quoted_len(tail, None),
            // 字符字面量；转义形式从被转义字符之后找收尾引号。其余 `'` 是生命周期或标签。
            ['\'', '\\', _, tail @ ..] => 4 + tail.iter().take_while(|&&c| c != '\'').count(),
            ['\'', _, '\'', ..] => 3,
            [':', ':', ..] => {
                tokens.push("::".to_owned());
                2
            }
            // 与 Rust 词法一致：紧挨着的 `!=` 是比较运算符，不是宏调用的 `!`。
            ['!', '=', ..] => {
                tokens.push("!=".to_owned());
                2
            }
            [c, ..] if is_word(*c) => word_len(rest, &mut tokens),
            [c, ..] => {
                tokens.push(c.to_string());
                1
            }
            [] => unreachable!("循环条件保证还有字符"),
        };
    }
    tokens
}

/// 块注释的长度（含嵌套）。
fn block_comment_len(rest: &[char]) -> usize {
    let (mut depth, mut index) = (0_usize, 0);
    while index < rest.len() {
        match &rest[index..] {
            ['/', '*', ..] => (depth, index) = (depth + 1, index + 2),
            ['*', '/', ..] if depth == 1 => return index + 2,
            ['*', '/', ..] => (depth, index) = (depth - 1, index + 2),
            _ => index += 1,
        }
    }
    index
}

/// 开引号之后到收尾引号（含）的长度；原始字符串带 `hashes` 个 `#` 收尾且不认转义。
fn quoted_len(rest: &[char], hashes: Option<usize>) -> usize {
    let mut index = 0;
    while let Some(&c) = rest.get(index) {
        match (c, hashes) {
            ('\\', None) => index += 2,
            ('"', None) => return index + 1,
            ('"', Some(count))
                if rest
                    .get(index + 1..index + 1 + count)
                    .is_some_and(|tail| tail.iter().all(|&c| c == '#')) =>
            {
                return index + 1 + count;
            }
            _ => index += 1,
        }
    }
    index
}

/// 标识符；`r`／`br`／`cr` 紧跟（可带 `#` 的）引号是原始字符串，`r#名字` 是原始标识符。
fn word_len(rest: &[char], tokens: &mut Vec<String>) -> usize {
    let len = rest.iter().take_while(|&&c| is_word(c)).count();
    let word = rest[..len].iter().collect::<String>();
    let hashes = rest[len..].iter().take_while(|&&c| c == '#').count();
    match (word.as_str(), rest.get(len + hashes)) {
        ("r" | "br" | "cr", Some('"')) => {
            len + hashes + 1 + quoted_len(&rest[len + hashes + 1..], Some(hashes))
        }
        // 只跳过 `r#`，名字在下一轮照常切出。
        ("r", Some(&c)) if hashes == 1 && is_word(c) => len + 1,
        _ => {
            tokens.push(word);
            len
        }
    }
}
