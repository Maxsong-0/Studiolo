//! stdin 和固定 JSON 的非秘密契约；不打开真实库、不运行撤权。
use super::*;

const FICTIONAL: &[u8] = b"3132333435363738393031323334353637383930";

#[test]
fn seed_is_exactly_forty_lowercase_hex_bytes_followed_by_eof() {
    assert!(read_seed(FICTIONAL).is_ok());
    for input in [
        Vec::new(),
        vec![b'1'; 39],
        vec![b'1'; 41],
        vec![b'A'; 40],
        vec![b'g'; 40],
        vec![255; 40],
        [FICTIONAL, b"\n"].concat(),
        [FICTIONAL, b"\0"].concat(),
    ] {
        let error = read_seed(input.as_slice()).err().unwrap();
        assert!(matches!(error, AdminError::RestoreInput));
        assert!(!format!("{error:?} {error}").contains(std::str::from_utf8(FICTIONAL).unwrap()));
    }
}

#[test]
fn partial_reads_are_supported_and_excess_input_is_bounded() {
    struct Bytewise<'a>(&'a [u8], usize);
    impl Read for Bytewise<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.1 == self.0.len() {
                return Ok(0);
            }
            out[0] = self.0[self.1];
            self.1 += 1;
            Ok(1)
        }
    }
    assert!(read_seed(Bytewise(FICTIONAL, 0)).is_ok());
    let mut reader = Bytewise(&[b'a'; 100], 0);
    assert!(read_seed(&mut reader).is_err());
    assert_eq!(reader.1, 41);
}

#[test]
fn audit_failure_still_outputs_only_fixed_non_secret_json_and_returns_error() {
    for confirmed in [true, false] {
        let report = RestoreInvalidation {
            epoch: 7,
            revoked_sessions: 2,
            cleared_l3: 1,
            invalidated_recovery_codes: 10,
            audit_confirmed: confirmed,
        };
        let mut output = Vec::new();
        let result = write_report(&report, &mut output);
        assert_eq!(result.is_ok(), confirmed);
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"epoch":7,"revoked_sessions":2,"cleared_l3":1,
            "invalidated_recovery_codes":10,"audit_confirmed":confirmed})
        );
        assert!(
            !String::from_utf8(output)
                .unwrap()
                .contains(std::str::from_utf8(FICTIONAL).unwrap())
        );
        if !confirmed {
            assert!(matches!(result, Err(AdminError::RestoreAuditPending)));
        }
    }
}

#[test]
fn input_and_output_io_errors_are_fixed_and_never_echoed() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("fictional-private-input"))
        }
    }
    let error = read_seed(Broken).err().unwrap();
    assert!(!format!("{error:?} {error}").contains("fictional-private-input"));
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("fictional-private-output"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let report = RestoreInvalidation {
        epoch: 1,
        revoked_sessions: 0,
        cleared_l3: 0,
        invalidated_recovery_codes: 0,
        audit_confirmed: true,
    };
    let error = write_report(&report, Broken).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("fictional-private-output"));
}
