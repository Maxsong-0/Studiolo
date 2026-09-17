# T-044 Rust 调用位置虚构示例

沿用 T-043 的三个虚构包和设计声明，添加三个显式源码文件；代码只供语法展示，
不保证可编译，也不执行。`ignored.rs` 刻意无效且不在白名单，`mod` 不会引入读取。

在 Studiolo 仓库根构建 web 和 studiolo-local-server，再运行：

```bash
target/debug/studiolo-local-server --vault examples/m3-rust-call-sites/vault \
  --assets web/dist --port 4327 \
  --project-repository "项目/示例工坊=$PWD/examples/m3-rust-call-sites/repository" \
  --cargo-dependencies \
  --rust-call-sites '["src/catalog/src/lib.rs","src/search/src/lib.rs","tools/src/lib.rs"]'
```

打开 [本机示例](http://127.0.0.1:4327/) 的“项目运作”，查看“Rust 调用位置”。
选择文件、词法上下文和调用种类，查看行/字节列，以及未展开宏、属性与外部模块。
文件下方列出按目录包含匹配的设计声明，可打开 Vault 原文；源码位置只显示文字。

`callback()` 不证明函数目标，闭包、async 和嵌套函数各有独立词法上下文；条件属性
不求值，宏 token tree 不提取内部调用。空结果也不证明没有实际调用。
两个开关独立：移除 Rust 开关只关闭源码证据，移除 Cargo 开关只关闭清单依赖。
源码改动后重新核验，Vault 声明改动后刷新工作台；源文件与 Vault 均不会被写入。
