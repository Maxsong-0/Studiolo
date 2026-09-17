// 只供未展开语法展示；不会执行此文件。
use crate::external::fetch as remote_fetch;
mod ignored;

pub fn build_catalog(callback: fn()) {
    collect();
    collect();
    r#collect();
    callback();
    remote_fetch();
    nested::run();
    let deferred = || collect();
}

fn collect() { normalize(); }
fn normalize() {}
fn unused() {}

pub fn shadowed() {
    let collect = || {};
    collect();
}

pub fn attribute_boundary() {
    #[cfg(feature = "optional")]
    fn conditional() {}
    conditional();
    collect();
}

mod nested {
    pub fn run() { collect(); }
    fn collect() {}
}
