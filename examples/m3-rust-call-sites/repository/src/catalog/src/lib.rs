// 纯语法示例：仅供只读位置提取，不构建或执行。
mod ignored;

pub struct Catalog;
impl Catalog {
    pub fn find(&self) {
        let key = normalize();
        self.lookup(key);
        let later = || deferred();
        let pending = async { remote().await; };
        fn nested() { inner(); }
        println!("参数内容不会输出", token_tree_call());
    }
}

#[cfg(feature = "fictional")]
pub fn conditional() { optional_path(); }

macro_rules! declared { () => { never_expanded() }; }
