# 图2虚构示例（T-041 / T-042）

`vault/` 和 `repository/` 都是虚构夹具。目录声明包含一个缺失目录，仓库另含一个
未文档化目录；`src/catalog/internal/` 未在文档展开，故不会被误报。
`项目运作.md` 增加五个设计节点、六条关系和两条任务路径，包含自环、重复到达
同一节点的步骤，以及存在、缺失和未文档化的关联目录。全部内容都是虚构设计。

在仓库根目录构建后运行：

```bash
npm --prefix web run build
cargo build --locked -p studiolo-local-server
cargo run --locked -p studiolo-local-server -- \
  --vault examples/m3-project-operations/vault --assets web/dist --port 4325 \
  --project-repository "项目/示例工坊=$PWD/examples/m3-project-operations/repository"
```

打开 `http://127.0.0.1:4325/` 的「项目运作」。此实例只读，不读取源码文件正文。
该图展示目录及文档声明的职责，不表示任何模块已部署或任务正在运行。
查看模块关系、切换依赖/数据流筛选，然后进入设计路径逐步查看“查询资料”或
“查询归档”。来源按钮返回表格原文；步骤推进不会启动任务或调用任何工具。
