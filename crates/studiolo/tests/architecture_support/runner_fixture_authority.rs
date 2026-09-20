//! Exact call inventories and boundary anchors for the final M6 implementation.
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/board/executor/socket_tests.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "letdirectory=tempfile::tempdir().unwrap();",
                "letpath=directory.path().join(\"runner.sock\");",
                "fnparent_replacement_cannot_redirect_cleanup()",
                "letparent=directory.path().join(\"original\");",
                "letmoved=directory.path().join(\"moved\");",
            ],
            &[
                ("UnixListener::bind(&path)", 3),
                ("std::fs::remove_file(&path)", 1),
                ("std::fs::create_dir(&parent)", 2),
                ("std::fs::rename(&parent,&moved)", 1),
            ],
        );
    }
    if path.ends_with("crates/local-server/src/board/executor/landing_tests.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "letroot=tempfile::tempdir().unwrap();",
                "letrepo=root.path().join(\"repo\");",
                "letstate=root.path().join(\"state\");",
                "letpatch_path=root.path().join(\"retained.patch\");",
                "assert!(Lease::acquire(&state,&repo).is_err());",
                "fndirty_and_untracked_baseline_cas_preserves_index_and_refs()",
            ],
            &[
                (
                    "Command::new(\"git\").env_clear().env(\"PATH\",\"/usr/bin:/bin\").env(\"GIT_CONFIG_GLOBAL\",\"/dev/null\").env(\"GIT_CONFIG_NOSYSTEM\",\"1\").args([\"-c\",\"user.name=Landingtest\",\"-c\",\"user.email=landing@invalid\",\"-C\"]).arg(repo).args(args).output()",
                    1,
                ),
                ("fs::create_dir(&repo)", 2),
                ("fs::canonicalize(repo)", 1),
                ("fs::create_dir(&state)", 1),
                ("fs::canonicalize(state)", 1),
                ("os::unix::fs::PermissionsExt", 1),
                ("fs::write(&patch_path,&patch)", 1),
                ("fs::write(&plan.patch_path,[])", 1),
                ("fs::Permissions::from_mode(0o700)", 1),
                ("fs::write(repo.join(\"a.txt\"),\"head\\n\")", 1),
                ("fs::write(repo.join(\"a.txt\"),\"candidate\\n\")", 1),
                ("fs::read(plan.repository.join(\".git/index\"))", 2),
                ("fs::write(repo.join(\"a.txt\"),\"dirtybaseline\\n\")", 2),
                ("fs::read_to_string(plan.repository.join(\"a.txt\"))", 2),
                ("fs::remove_file(plan.repository.join(\"late.txt\"))", 1),
                (
                    "fs::write(repo.join(\"untracked.txt\"),\"approvedinput\\n\")",
                    1,
                ),
                (
                    "fs::write(plan.repository.join(\"late.txt\"),\"humanedit\\n\")",
                    1,
                ),
                (
                    "fs::set_permissions(&state,fs::Permissions::from_mode(0o700))",
                    1,
                ),
                (
                    "fs::write(plan.repository.join(\"untracked.txt\"),\"laterchange\\n\")",
                    1,
                ),
            ],
        );
    }
    source.to_owned()
}
