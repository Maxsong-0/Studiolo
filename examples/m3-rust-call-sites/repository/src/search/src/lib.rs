// 这里的名称没有经过 Rust 类型或符号解析。
pub fn query() {
    crate::validate::<u8>();
    let callback = || prepare();
    callback();
    (callback)();
    factory()();
    tracing::event!("只记录宏路径，不输出参数", hidden());
}

pub mod formatting {
    pub fn present() { render(); }
}

const VALUE: usize = const { compute() };
static STATE: usize = initialize();
