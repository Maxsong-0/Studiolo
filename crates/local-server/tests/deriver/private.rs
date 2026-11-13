//! 专用派生线程自身的约束（T-186，vault BUG-035）：凭证带不出这条线程；在它自己的任务里
//! 同步等它会被直接拒绝，而不是等自己、永远挂住。

use std::sync::Arc;

use crate::{
    LocalError,
    deriver::{Deriver, OnDeriveThread},
};

/// 凭证若实现了 `Send`，下面两个实现都适用于它，`_` 推断不出唯一的类型，编译失败。
trait AmbiguousIfSend<A> {
    fn check() {}
}
impl<T: ?Sized> AmbiguousIfSend<()> for T {}
impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

/// 同上，针对 `Sync`：`&OnDeriveThread` 能不能交给别的线程，取决于它。
trait AmbiguousIfSync<A> {
    fn check() {}
}
impl<T: ?Sized> AmbiguousIfSync<()> for T {}
impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}

#[test]
fn the_derivation_token_cannot_cross_threads() {
    // 这两行能编译本身就是断言：凭证既不能转移到、也不能借给别的线程，整库建模因此离不开专用线程。
    <OnDeriveThread as AmbiguousIfSend<_>>::check();
    <OnDeriveThread as AmbiguousIfSync<_>>::check();
}

#[test]
fn waiting_for_the_derivation_thread_from_its_own_job_is_refused() {
    let deriver = Arc::new(Deriver::start().unwrap());
    let inner = Arc::clone(&deriver);
    let nested = deriver
        .run_blocking(move |_| inner.run_blocking(|_| ()))
        .unwrap();
    assert!(
        matches!(nested, Err(LocalError::Unavailable)),
        "在专用线程自己的任务里同步等它只会等自己，必须直接拒绝"
    );
    // 拒绝之后线程照常接活。
    assert_eq!(deriver.run_blocking(|_| 7).unwrap(), 7);
}
