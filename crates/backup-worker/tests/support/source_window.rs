use super::*;
fn ms(value: i64) -> InstantMillis {
    InstantMillis::from_millis(value)
}

#[test]
fn retention_caps_original_budget_forward_jumps_shorten_and_never_reset() {
    let start = Instant::now();
    let window = Window::new(
        start + Duration::from_secs(100),
        start,
        ms(1000),
        ms(900),
        ms(5000),
    )
    .unwrap();
    assert_eq!(window.deadline(), start + Duration::from_secs(4));
    window
        .observe(ms(2000), start + Duration::from_secs(1))
        .unwrap();
    assert_eq!(window.deadline(), start + Duration::from_secs(4));
    window
        .observe(ms(4500), start + Duration::from_secs(2))
        .unwrap();
    assert_eq!(window.deadline(), start + Duration::from_millis(2500));
    assert_eq!(
        window.observe(ms(4500), start + Duration::from_millis(2500)),
        Err(ProcessError::Deadline)
    );
    assert_eq!(window.observe(ms(4600), start), Err(ProcessError::Deadline));
}
#[test]
fn rollback_expiry_future_cutoff_overflow_and_excess_budget_refuse() {
    let start = Instant::now();
    for (now, cutoff, expiry, seconds) in [
        (1000, 1001, 5000, 10),
        (5000, 900, 5000, 10),
        (1000, 900, 5000, 7201),
        (i64::MIN, i64::MIN, i64::MAX, 10),
    ] {
        assert!(
            Window::new(
                start + Duration::from_secs(seconds),
                start,
                ms(now),
                ms(cutoff),
                ms(expiry)
            )
            .is_err()
        );
    }
    let window = Window::new(
        start + Duration::from_secs(10),
        start,
        ms(1000),
        ms(900),
        ms(5000),
    )
    .unwrap();
    assert_eq!(window.observe(ms(999), start), Err(ProcessError::Deadline));
    assert_eq!(window.observe(ms(1001), start), Err(ProcessError::Deadline));
    assert!(utc_now().unwrap().millis() > 0);
}
