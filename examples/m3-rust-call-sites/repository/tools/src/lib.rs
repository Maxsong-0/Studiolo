// 调用位置只表示词法包含，不是执行顺序。
pub trait Export {
    fn required(&self);
    fn default_export(&self) { self.write(); }
}

pub fn 导出() {
    serialize();
    let deferred = || { save(); };
}
