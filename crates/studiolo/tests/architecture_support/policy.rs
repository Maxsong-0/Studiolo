pub(crate) fn allowed_workspace_dependencies(crate_name: &str) -> &'static [&'static str] {
    match crate_name {
        "studiolo-core" => &[],
        "studiolo-sync-bridge" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-backup" | "studiolo-observability" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-sync-transport" => &["studiolo-core", "studiolo-host-config"],
        "studiolo-vault-broker"
        | "studiolo-vault-index"
        | "studiolo-control-db"
        | "studiolo-audit-ledger"
        | "studiolo-runner-adapter" => &["studiolo-core"],
        "studiolo-gateway" => &["studiolo-core", "studiolo-audit-ledger"],
        // M4 策略平面：只依赖值类型与审计事件类型；不得依赖 control-db / auth / vault-broker。
        "studiolo-policy" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-auth" => &["studiolo-audit-ledger"],
        "studiolo-host-config" => &["studiolo-auth", "studiolo-core"],
        "studiolo-auth-admin" => &[
            "studiolo-auth",
            "studiolo-audit-ledger",
            "studiolo-host-config",
        ],
        "studiolo-workbench" => &["studiolo-core", "studiolo-vault-index"],
        "studiolo-local-server" => &[
            "studiolo-auth",
            "studiolo-audit-ledger",
            "studiolo-host-config",
            "studiolo-core",
            "studiolo-vault-broker",
            "studiolo-vault-index",
            "studiolo-workbench",
            "studiolo-backup",
            "studiolo-observability",
            "studiolo-sync-transport",
            // M4：人侧发布者句柄（L3 之后写 effective）与三级吊销入口。
            "studiolo-policy",
        ],
        "studiolo-task-engine" => &[
            "studiolo-core",
            "studiolo-control-db",
            "studiolo-vault-broker",
            "studiolo-audit-ledger",
            "studiolo-runner-adapter",
        ],
        "studiolo" => &[
            "studiolo-core",
            "studiolo-vault-broker",
            "studiolo-vault-index",
            "studiolo-control-db",
            "studiolo-audit-ledger",
            "studiolo-runner-adapter",
            "studiolo-gateway",
            "studiolo-task-engine",
        ],
        // M3 供给面：只读 Broker 能力 + 索引/搜索 + 审计追加；不得依赖 control-db / auth / task-engine。
        "studiolo-supply" => &[
            "studiolo-core",
            "studiolo-vault-broker",
            "studiolo-vault-index",
            "studiolo-workbench",
            "studiolo-audit-ledger",
        ],
        // MCP 主外壳：只经 supply 取内容；不直接依赖 vault-broker / control-db / auth。
        // M4 加 policy：接入方句柄（读 effective 与吊销状态、只插 pending 提案）。
        "studiolo-agent-server" => &[
            "studiolo-supply",
            "studiolo-audit-ledger",
            "studiolo-core",
            "studiolo-policy",
        ],
        other => panic!("new workspace crate {other} needs an explicit dependency policy"),
    }
}
