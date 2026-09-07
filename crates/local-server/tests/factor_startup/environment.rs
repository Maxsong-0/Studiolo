//! 固定双槽的纯输入测试，不修改进程环境，不读取真实秘密。

use super::*;

const HEX: &str = "3132333435363738393031323334353637383930";
const NEXT: &str = "3039383736353433323130393837363534333231";

#[test]
fn environment_slots_require_both_replacement_reference_and_secret() {
    let (_, replacement) = slots("current", None, Some(HEX.into()), None).unwrap();
    assert!(replacement.is_none());
    let (_, replacement) =
        slots("current", Some("next"), Some(HEX.into()), Some(NEXT.into())).unwrap();
    assert!(replacement.is_some());
    for (reference, value) in [(None, Some(NEXT.into())), (Some("next"), None)] {
        assert!(matches!(
            slots("current", reference, Some(HEX.into()), value),
            Err(StartupError::FactorEnvironment)
        ));
    }
}

#[test]
fn missing_or_invalid_either_slot_fails_without_echoing_material() {
    for value in [
        None,
        Some("secret-canary".into()),
        Some("A".repeat(40).into()),
        Some(OsString::from_vec(vec![255; 40])),
        Some(format!("{HEX}\n").into()),
    ] {
        let current = slots("current", None, value.clone(), None).err().unwrap();
        let replacement = slots("current", Some("next"), Some(HEX.into()), value)
            .err()
            .unwrap();
        for error in [current, replacement] {
            assert!(matches!(error, StartupError::FactorEnvironment));
            assert!(!format!("{error:?} {error}").contains("secret-canary"));
        }
    }
    for reference in ["", "../reference", &"x".repeat(65)] {
        assert!(slots(reference, None, Some(HEX.into()), None).is_err());
        assert!(
            slots(
                "current",
                Some(reference),
                Some(HEX.into()),
                Some(NEXT.into())
            )
            .is_err()
        );
    }
}

#[test]
fn factor_process_core_limits_are_closed_and_can_only_be_tightened() {
    disable_core_dumps().unwrap();
    disable_core_dumps().unwrap();
    let limits = getrlimit(Resource::Core);
    assert_eq!(limits.current, Some(0));
    assert_eq!(limits.maximum, Some(0));
}
