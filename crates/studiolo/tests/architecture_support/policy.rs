pub(crate) fn allowed_workspace_dependencies(crate_name: &str) -> &'static [&'static str] {
    match crate_name {
        "studiolo-core" => &[],
        // 秘密驻留面加固：只依赖 rustix，零工作区依赖。四个持明文秘密的程序都要能
        // 无条件依赖它，包括不得依赖认证栈的 studiolo-agent-server；它一旦长出任何
        // 工作区依赖，那条「谁都能用」的性质就没了，加固又会漏掉某个程序。
        "studiolo-process-safety" => &[],
        "studiolo-sync-bridge" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-backup" | "studiolo-observability" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-sync-transport" => &["studiolo-core", "studiolo-host-config"],
        // T-132 P1（ADR-025 D2）：可信 launcher 只认线上契约与值类型；不得依赖 host-config／vault-broker。
        "studiolo-sync-launcher" => &["studiolo-sync-transport", "studiolo-core"],
        "studiolo-vault-broker"
        | "studiolo-vault-index"
        | "studiolo-control-db"
        | "studiolo-audit-ledger"
        | "studiolo-runner-adapter" => &["studiolo-core"],
        "studiolo-gateway" => &[
            "studiolo-core",
            "studiolo-audit-ledger",
            "studiolo-process-safety",
        ],
        // M4 策略平面：只依赖值类型与审计事件类型；不得依赖 control-db / auth / vault-broker。
        "studiolo-policy" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-auth" => &["studiolo-audit-ledger"],
        "studiolo-host-config" => &["studiolo-auth", "studiolo-core"],
        "studiolo-auth-admin" => &[
            "studiolo-auth",
            "studiolo-audit-ledger",
            "studiolo-host-config",
            "studiolo-process-safety",
        ],
        // 离线首建只组合既有配置、只读源、Broker 与审计，不获得身份签发/策略发布。
        "studiolo-vault-admin" => &[
            "studiolo-core",
            "studiolo-vault-broker",
            "studiolo-host-config",
            "studiolo-audit-ledger",
            "studiolo-process-safety",
        ],
        "studiolo-workbench" => &["studiolo-core", "studiolo-vault-index"],
        "studiolo-local-server" => &[
            "studiolo-runner-adapter",
            "studiolo-gateway",
            "studiolo-control-db",
            "studiolo-task-engine",
            "studiolo-supply",
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
            // T-132 P1（ADR-025 D1）：Bridge 驱动平面装进本进程，纯执行器与持久状态由它组合。
            "studiolo-sync-bridge",
            // M4：人侧发布者句柄（L3 之后写 effective）与三级吊销入口。
            "studiolo-policy",
            "studiolo-process-safety",
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
        // process-safety 是唯一的例外方向：它零工作区依赖，接上它不会把认证栈带进供给面，
        // 而它是本程序持有的类型 token 与 bearer 明文唯一的驻留面收紧来源。
        "studiolo-agent-server" => &[
            "studiolo-supply",
            "studiolo-audit-ledger",
            "studiolo-core",
            "studiolo-policy",
            "studiolo-process-safety",
        ],
        other => panic!("new workspace crate {other} needs an explicit dependency policy"),
    }
}
