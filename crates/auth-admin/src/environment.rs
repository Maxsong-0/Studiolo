//! 固定环境槽位；只零化本进程拥有的副本，不宣称擦除操作系统环境。

use std::{ffi::OsString, os::unix::ffi::OsStringExt};
use studiolo_auth::{SessionError, TotpSeed};
use zeroize::Zeroizing;

pub(crate) const SEED_VARIABLE: &str = "STUDIOLO_TOTP_SEED_HEX";

pub(crate) fn read_seed(reference: String) -> Result<TotpSeed, crate::AdminError> {
    decode(reference, std::env::var_os(SEED_VARIABLE))
}

fn decode(reference: String, value: Option<OsString>) -> Result<TotpSeed, crate::AdminError> {
    let value = value.ok_or(crate::AdminError::Environment)?;
    let encoded = Zeroizing::new(value.into_vec());
    TotpSeed::from_hex(reference, &encoded).map_err(|error| match error {
        SessionError::Rejected => crate::AdminError::Environment,
        error => crate::AdminError::Session(error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_adapter_preserves_missing_encoding_and_reference_errors() {
        let valid = "3132333435363738393031323334353637383930";
        assert!(decode("fixture-v1".into(), Some(valid.into())).is_ok());
        for value in [
            None,
            Some("A".repeat(40).into()),
            Some(OsString::from_vec(vec![255; 40])),
        ] {
            assert!(matches!(
                decode("fixture-v1".into(), value),
                Err(crate::AdminError::Environment)
            ));
        }
        assert!(matches!(
            decode("../seed".into(), Some(valid.into())),
            Err(crate::AdminError::Session(SessionError::Configuration))
        ));
        assert!(matches!(
            decode("../seed".into(), Some("invalid-encoding".into())),
            Err(crate::AdminError::Environment)
        ));
    }

    #[test]
    fn errors_do_not_echo_the_value() {
        let secret = "not-an-actual-secret-please-do-not-echo";
        let error = decode("fixture".into(), Some(secret.into())).unwrap_err();
        assert!(!format!("{error:?} {error}").contains(secret));
    }
}
