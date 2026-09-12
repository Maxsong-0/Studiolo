pub fn start() {
    query();
    let deferred = || query();
    let future = async { query(); };
}

fn query() { decode(); }
fn decode() {}

pub fn with_macro() {
    println!("宏内容不进入调用索引");
    query();
}

pub fn with_match(input: Option<fn()>) {
    match input {
        Some(query) => query(),
        None => query(),
    }
}

pub fn generic_calls() {
    generic::<u8>();
    generic();
}
fn generic<T>() {}
