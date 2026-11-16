use super::*;
#[test]
fn identity_root_cannot_be_served_or_overlap_private_data() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    for name in ["vault", "web", "policy", "identities", "auth"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    let args = [
        "--vault".into(),
        root.join("vault").to_string_lossy().into_owned(),
        "--assets".into(),
        root.join("web").to_string_lossy().into_owned(),
        "--session-config".into(),
        root.join("owner.json").to_string_lossy().into_owned(),
        "--public-origin".into(),
        "https://example.com".into(),
        "--policy-data".into(),
        root.join("policy").to_string_lossy().into_owned(),
    ];
    let options = crate::options::parse(args.into_iter()).unwrap().unwrap();
    let protected = [root.join("auth")];
    let paths = identity_isolated_roots(&options, &protected);
    assert_eq!(
        paths,
        [
            root.join("vault"),
            root.join("web"),
            root.join("policy"),
            root.join("auth")
        ]
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>()
    );
}
