# T-043 Cargo清单依赖虚构示例

本示例沿用T-042虚构项目设计说明，另有独立Cargo根和三个虚构包。依赖含rename、
workspace继承、optional、target/build和dev循环；外部依赖只计条数。目录中的
归档仍刻意缺失，tools仍未文档化；Cargo图不改变这些独立证据。

在Studiolo仓库根构建web和studiolo-local-server，再运行：

```bash
target/debug/studiolo-local-server --vault examples/m3-cargo-dependencies/vault \
  --assets web/dist --port 4326 \
  --project-repository "项目/示例工坊=$PWD/examples/m3-cargo-dependencies/repository" \
  --cargo-dependencies
```

打开http://127.0.0.1:4326/的“项目运作”，查看“Cargo清单依赖”：三个包、五条内部
依赖、两条外部依赖声明。筛选普通/开发/构建依赖，查看目标条件、可选标记、成员
清单行及workspace继承定义行；选包后可打开按目录精确关联的设计节点原文。
移除启动开关后Cargo能力关闭，已有目录和设计图继续可用。修改清单后重新核验，
修改Vault设计声明后刷新工作台。这里没有实际调用或任务执行证据。
