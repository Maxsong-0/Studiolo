use super::SerialGate;

/// 持有者 panic 之后，后续进入者必须能接管闸门；否则一次真失败会级联成一片
/// `PoisonError`，把根因淹没（T-049 回归）。
#[test]
fn a_poisoned_gate_is_taken_over_by_the_next_entrant() {
    static GATE: SerialGate = SerialGate::new();
    let failed = std::panic::catch_unwind(|| {
        let _guard = GATE.enter();
        panic!("T-049 回归用例故意在持有闸门时失败，本条 panic 属预期");
    });
    assert!(failed.is_err(), "构造的 panic 未发生，回归前提不成立");
    assert!(GATE.0.lock().is_err(), "闸门未被毒化，回归前提不成立");
    let _recovered = GATE.enter();
}
