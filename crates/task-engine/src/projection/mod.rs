//! 任务页投影只替换四个顶层标量，其他字节归作者所有。

pub(crate) mod key;
mod page;
mod patch;

pub(crate) use page::{render_task_page, validate_task_page};
pub(crate) use patch::apply_projection;
