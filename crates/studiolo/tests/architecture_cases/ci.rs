use crate::support::workspace_root;
use std::fs;

#[test]
fn ci_commands_lock_dependencies_and_run_linux_path_security_explicitly() {
    let root = workspace_root();
    for relative in [".github/workflows/ci.yml", "scripts/test-linux.sh"] {
        let content = fs::read_to_string(root.join(relative)).expect("read CI entrypoint");
        for command in [
            "cargo --locked fmt",
            "cargo --locked clippy",
            "cargo --locked test",
            "cargo --locked test -p studiolo-vault-broker --test linux_path_security",
        ] {
            assert!(
                content.contains(command),
                "{relative} is missing `{command}`"
            );
        }
    }
}
