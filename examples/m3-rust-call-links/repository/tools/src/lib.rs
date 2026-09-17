pub fn run() {
    r#inspect();
    inspect();
    café();
    child::run();
}
fn inspect() { inspect(); }
fn café() {}

mod child {
    pub fn run() { inspect(); }
}
