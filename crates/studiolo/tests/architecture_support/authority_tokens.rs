//! Token-local exceptions: exact count and safety anchors precede any removal.
pub(super) fn compact(source: &str) -> String {
    source
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")")
        .replace(",]", "]")
        .replace(",}", "}")
}
pub(super) fn checked(source: &str, anchors: &[&str], calls: &[(&str, usize)]) -> String {
    let original = compact(source);
    if anchors.iter().any(|anchor| !original.contains(anchor))
        || calls
            .iter()
            .any(|(call, count)| original.matches(call).count() != *count)
    {
        return original;
    }
    let mut remaining = original;
    let mut ordered = calls.to_vec();
    ordered.sort_by_key(|(call, _)| std::cmp::Reverse(call.len()));
    for (call, count) in ordered {
        remaining = remaining.replacen(call, "", count);
    }
    remaining
}

/// These new sites also use imported rustix/socket calls: keep their complete call inventory closed.
pub(super) fn checked_closed(source: &str, anchors: &[&str], calls: &[(&str, usize)]) -> String {
    let remainder = checked(source, anchors, calls);
    let extra = [
        "fs::",
        "Command::new(",
        "File::open(",
        "File::from(",
        "File::options(",
        "OpenOptions::new(",
        ".write_all(",
        ".sync_all(",
        "TcpStream::connect",
        "UnixListener::bind(",
        "Connection::open",
        "tempfile::NamedTempFile::new_in(",
        ".persist(",
        ".persist_noclobber(",
    ]
    .iter()
    .any(|token| remainder.contains(token));
    let bare = ["open", "unlinkat", "chmodat", "statat", "flock"]
        .iter()
        .any(|name| {
            let needle = format!("{name}(");
            remainder.match_indices(needle.as_str()).any(|(offset, _)| {
                offset == 0
                    || !remainder.as_bytes()[offset - 1].is_ascii_alphanumeric()
                        && !matches!(remainder.as_bytes()[offset - 1], b'_' | b':' | b'.')
            })
        });
    if extra || bare {
        compact(source)
    } else {
        remainder
    }
}
