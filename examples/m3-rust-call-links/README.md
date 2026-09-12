# T-045 Rust 同文件函数词法关联示例

三个显式文件展示同文件自由函数关联、反向引用与受控未解析原因。沿用虚构目录和
设计声明；代码只供语法展示，不保证可编译，也不执行。ignored.rs 刻意无效且不在
白名单，mod 不会引入读取。运行前在 Studiolo 仓库根构建 web 和 local-server。

```bash
target/debug/studiolo-local-server --vault examples/m3-rust-call-links/vault \
  --assets web/dist --port 4328 \
  --project-repository "项目/示例工坊=$PWD/examples/m3-rust-call-links/repository" \
  --cargo-dependencies \
  --rust-call-sites '["src/catalog/src/lib.rs","src/search/src/lib.rs","tools/src/lib.rs"]'
```

打开 [本机示例](http://127.0.0.1:4328/) 的“项目运作 → Rust 调用位置”：

- catalog 的 collect 有多处同批引用；build_catalog 内 raw identifier 同样关联。
- unused 有定义但没有本片关联引用；这不能证明它在整个程序中未被使用。
- callback、remote_fetch、shadowed 保留同名遮蔽原因；限定路径不会跨模块猜测。
- attribute_boundary 和 with_macro 的范围不透明；方法、泛型等保持未解析。
- tools 的 inspect 递归可关联，child 中的裸 inspect 不向父模块查找。

点击关联项查看函数定义位置，再反查该函数的调用位置；来源文字使用一基行/字节列。
词法关联不等于编译器目标解析或运行轨迹。没有源码正文读取入口，既有设计声明的
Vault 来源按钮仍可打开笔记。两轮捕获及解析均只读；重新核验会清空旧选择。

规则与预算见 [冻结契约](../../docs/m3/rust-call-links.md)。生产进程隔离、完整符号图
和真实运行全链路仍未验收。本例只用于本地独立验证，不修改已有服务。

构建后运行 `python3 examples/m3-rust-call-links/check.py`，会在临时端口分别检查
虚构示例与 Studiolo 自身两个显式文件，独立核对批哈希、关联引用、来源字节位置和
文件只读指纹，并停止它自己启动的进程。
