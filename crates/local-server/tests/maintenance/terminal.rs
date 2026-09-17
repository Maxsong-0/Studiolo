use super::super::terminal::{Input, validate_ids};

#[test]
fn actual_uid_and_gid_must_match_effective_identity() {
    let actual = validate_ids(501, 501, 20, 20).unwrap();
    assert_eq!(actual.uid, 501);
    assert_eq!(actual.gid, 20);
    assert!(validate_ids(501, 0, 20, 20).is_err());
    assert!(validate_ids(501, 501, 20, 0).is_err());
}

#[test]
fn terminal_confirmation_is_one_bounded_ascii_line() {
    let mut input = Input::default();
    assert!(input.push(b"cancel-prepared ").unwrap().is_none());
    assert_eq!(
        input.push(b"abc\n").unwrap().as_deref(),
        Some("cancel-prepared abc")
    );
    for bytes in [b"a\nb\n".as_slice(), b"a\r", b"a\x1b", b"a\x04", b"\xff"] {
        assert!(Input::default().push(bytes).is_err());
    }
    assert!(Input::default().push(&[b'a'; 129]).is_err());
    let mut input = Input::default();
    assert!(input.push(&[b'a'; 127]).unwrap().is_none());
    assert!(input.push(b"a\n").is_err());
}
