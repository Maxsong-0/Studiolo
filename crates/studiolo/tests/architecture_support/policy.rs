pub(crate) fn allowed_workspace_dependencies(crate_name: &str) -> &'static [&'static str] {
    match crate_name {
        "studiolo-core" => &[],
        "studiolo-vault-broker"
        | "studiolo-vault-index"
        | "studiolo-control-db"
        | "studiolo-audit-ledger"
        | "studiolo-runner-adapter" => &["studiolo-core"],
        "studiolo-gateway" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-auth" => &["studiolo-audit-ledger"],
        "studiolo-host-config" => &["studiolo-auth"],
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
        other => panic!("new workspace crate {other} needs an explicit dependency policy"),
    }
}
