# 图2首片虚构示例

`vault/` 和 `repository/` 都是虚构夹具。目录声明包含一个缺失目录，仓库另含一个
未文档化目录；`src/catalog/internal/` 未在文档展开，故不会被误报。

在仓库根目录构建后运行：

```bash
npm --prefix web run build
cargo build --locked -p studiolo-local-server
cargo run --locked -p studiolo-local-server -- \
  --vault examples/m3-project-operations/vault --assets web/dist --port 4324 \
  --project-repository "项目/示例工坊=$PWD/examples/m3-project-operations/repository"
```

打开 `http://127.0.0.1:4324/` 的「项目运作」。此实例只读，不读取源码文件正文。
该图展示目录及文档声明的职责，不表示任何模块已部署或任务正在运行。
