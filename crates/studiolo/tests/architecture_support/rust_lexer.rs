//! 按源码文本做判定的守卫共用的 Rust 记号化：只切出标识符与标点，
//! 空白、注释、字符串与字符字面量一概不产生记号，守卫因此不会被它们误导或绕开。
//! 要认字面量写法的守卫（如 `b'a'..=b'f'`、`"{b:02x}"`）用 [`lex_with_literals`]：字面量各留成一个原样的记号。

pub(crate) fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// 切成标识符与标点记号：跳过空白、注释（块注释可嵌套）、字符串与字符字面量，
/// `::` 与 `!=` 各合成一个记号，原始标识符 `r#名字` 记作 `名字`。
pub(crate) fn lex(source: &str) -> Vec<String> {
    tokenize(source, false)
}

/// 同 [`lex`]，但字符串与字符字面量不跳过，各留成一个原样的记号（含引号；原始字符串连 `r`／`br` 前缀一起）。
/// 字面量里的 `{`、`//` 不会被当成标点或注释，测试模块照样能按括号配对去掉。
pub(crate) fn lex_with_literals(source: &str) -> Vec<String> {
    tokenize(source, true)
}

fn tokenize(source: &str, literals: bool) -> Vec<String> {
    let chars = source.chars().collect::<Vec<_>>();
    let (mut tokens, mut index) = (Vec::new(), 0);
    while index < chars.len() {
        let rest = &chars[index..];
        index += match rest {
            [c, ..] if c.is_whitespace() => 1,
            ['/', '/', ..] => rest.iter().take_while(|&&c| c != '\n').count(),
            ['/', '*', ..] => block_comment_len(rest),
            ['"', tail @ ..] => keep(literals, rest, 1 + quoted_len(tail, None), &mut tokens),
            // 字符字面量；转义形式从被转义字符之后找收尾引号。其余 `'` 是生命周期或标签。
            ['\'', '\\', _, tail @ ..] => {
                let len = 4 + tail.iter().take_while(|&&c| c != '\'').count();
                keep(literals, rest, len, &mut tokens)
            }
            ['\'', _, '\'', ..] => keep(literals, rest, 3, &mut tokens),
            [':', ':', ..] => {
                tokens.push("::".to_owned());
                2
            }
            // 与 Rust 词法一致：紧挨着的 `!=` 是比较运算符，不是宏调用的 `!`。
            ['!', '=', ..] => {
                tokens.push("!=".to_owned());
                2
            }
            [c, ..] if is_word(*c) => word_len(rest, literals, &mut tokens),
            [c, ..] => {
                tokens.push(c.to_string());
                1
            }
            [] => unreachable!("循环条件保证还有字符"),
        };
    }
    tokens
}

/// 字面量的长度；`literals` 为真时把它原样留成一个记号。
fn keep(literals: bool, rest: &[char], len: usize, tokens: &mut Vec<String>) -> usize {
    if literals {
        tokens.push(rest[..len.min(rest.len())].iter().collect());
    }
    len
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
fn word_len(rest: &[char], literals: bool, tokens: &mut Vec<String>) -> usize {
    let len = rest.iter().take_while(|&&c| is_word(c)).count();
    let word = rest[..len].iter().collect::<String>();
    let hashes = rest[len..].iter().take_while(|&&c| c == '#').count();
    match (word.as_str(), rest.get(len + hashes)) {
        ("r" | "br" | "cr", Some('"')) => {
            let raw = len + hashes + 1 + quoted_len(&rest[len + hashes + 1..], Some(hashes));
            keep(literals, rest, raw, tokens)
        }
        // 只跳过 `r#`，名字在下一轮照常切出。
        ("r", Some(&c)) if hashes == 1 && is_word(c) => len + 1,
        _ => {
            tokens.push(word);
            len
        }
    }
}

/// 产品代码的记号：只去掉 `#[cfg(test)]` 的测试模块本身（声明或内联）。测试模块的声明常在文件中间，
/// 从那里一刀切到文件末尾会漏掉后面的产品代码。
pub(crate) fn lex_product(source: &str) -> Vec<String> {
    without_test_modules(&lex(source))
}

/// 同 [`lex_product`]，字面量照 [`lex_with_literals`] 留着。
pub(crate) fn lex_product_with_literals(source: &str) -> Vec<String> {
    without_test_modules(&lex_with_literals(source))
}

fn without_test_modules(tokens: &[String]) -> Vec<String> {
    let (mut product, mut index) = (Vec::new(), 0);
    while index < tokens.len() {
        match test_module_len(&tokens[index..]) {
            Some(len) => index += len,
            None => {
                product.push(tokens[index].clone());
                index += 1;
            }
        }
    }
    product
}

/// 从 `#[cfg(test)]` 起到测试模块收尾的记号数；别的 `cfg(test)` 项不算，照样当产品代码看。
fn test_module_len(tokens: &[String]) -> Option<usize> {
    const TEST_ONLY: [&str; 7] = ["#", "[", "cfg", "(", "test", ")", "]"];
    if *tokens.get(..TEST_ONLY.len())? != TEST_ONLY {
        return None;
    }
    let mut index = TEST_ONLY.len();
    while tokens.get(index)? == "#" {
        index += 1 + enclosed(&tokens[index + 1..], "[", "]").len();
    }
    if tokens.get(index)? == "pub" {
        index += 1;
        if tokens.get(index)? == "(" {
            index += enclosed(&tokens[index..], "(", ")").len();
        }
    }
    if tokens.get(index)? != "mod" {
        return None;
    }
    let body = index + 2;
    match tokens.get(body)?.as_str() {
        ";" => Some(body + 1),
        "{" => Some(body + enclosed(&tokens[body..], "{", "}").len()),
        _ => None,
    }
}

/// 以 `open` 开头的记号串里，到配对的 `close` 为止（含两端）；没配上就到末尾。
pub(crate) fn enclosed<'a>(tokens: &'a [String], open: &str, close: &str) -> &'a [String] {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if token == open {
            depth += 1;
        } else if token == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return &tokens[..=index];
            }
        }
    }
    tokens
}

/// 头部之后第一个 `{` 到配对 `}` 的块体。
pub(crate) fn block(tokens: &[String]) -> &[String] {
    let start = tokens
        .iter()
        .position(|token| token == "{")
        .unwrap_or(tokens.len());
    enclosed(&tokens[start..], "{", "}")
}

#[cfg(test)]
mod tests {
    use super::{lex, lex_product, lex_product_with_literals, lex_with_literals};

    /// 字面量只在要的时候留下，且整个留成一个记号；里面的括号和 `//` 不影响切分。
    #[test]
    fn literals_are_kept_whole_only_when_asked() {
        let source = "let a = b'a'; let s = \"{b:02x} // }\"; x // c";
        assert_eq!(lex(source).concat(), "leta=b;lets=;x");
        assert_eq!(
            lex_with_literals(source).join(" "),
            "let a = b 'a' ; let s = \"{b:02x} // }\" ; x"
        );
        let raw = "let r = r#\"x\"y\"#; let e = '\\'';";
        assert_eq!(lex(raw).concat(), "letr=;lete=;");
        assert_eq!(
            lex_with_literals(raw).join(" "),
            "let r = r#\"x\"y\"# ; let e = '\\'' ;"
        );
        let tested = "#[cfg(test)] mod t { const S: &str = \"}\"; } fn a() { \"{\" }";
        assert_eq!(lex_product_with_literals(tested).concat(), "fna(){\"{\"}");
    }

    /// 只去测试模块自己：后面的产品代码、别的 `cfg(test)` 项、条件不只是 `test` 的模块都留着。
    #[test]
    fn only_the_test_modules_themselves_are_dropped() {
        for (source, expected) in [
            ("#[cfg(test)] mod t; fn a() {}", "fna(){}"),
            ("#[cfg(test)] mod t { fn b() {} } fn a() {}", "fna(){}"),
            (
                "#[cfg(test)] #[path = \"x.rs\"] mod t; fn a() {}",
                "fna(){}",
            ),
            ("#[cfg(test)] pub mod t; fn a() {}", "fna(){}"),
            ("#[cfg(test)] pub(crate) mod t; fn a() {}", "fna(){}"),
            ("fn a() {} #[cfg(test)] mod t;", "fna(){}"),
            ("#[cfg(test)] fn b() {}", "#[cfg(test)]fnb(){}"),
            ("#[cfg(test)] use b;", "#[cfg(test)]useb;"),
            ("#[cfg(test)] struct B {}", "#[cfg(test)]structB{}"),
            ("#[cfg(unix)] mod t;", "#[cfg(unix)]modt;"),
            (
                "#[cfg(any(test, unix))] mod t;",
                "#[cfg(any(test,unix))]modt;",
            ),
            ("#[cfg(not(test))] mod t;", "#[cfg(not(test))]modt;"),
            ("fn a() {} #[cfg(test)]", "fna(){}#[cfg(test)]"),
        ] {
            assert_eq!(lex_product(source).concat(), expected, "{source}");
        }
    }
}
