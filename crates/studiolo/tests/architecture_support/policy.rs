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
        "studiolo-agent-server" => &["studiolo-supply", "studiolo-audit-ledger", "studiolo-core"],
        other => panic!("new workspace crate {other} needs an explicit dependency policy"),
    }
}
