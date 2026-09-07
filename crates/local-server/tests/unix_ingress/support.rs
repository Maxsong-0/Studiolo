use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct Fixture {
    _temporary: tempfile::TempDir,
    pub root: PathBuf,
    pub parent: PathBuf,
    pub socket: PathBuf,
    pub isolated: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        // macOS 的 /tmp 是链接；夹具先取真实路径，生产 Interface 不自动追随链接。
        let temporary = tempfile::Builder::new()
            .prefix("stu-unix-")
            .tempdir_in(Path::new("/tmp").canonicalize().unwrap())
            .unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let parent = root.join("run");
        let isolated = root.join("data");
        for path in [&parent, &isolated] {
            fs::create_dir(path).unwrap();
            mode(path, 0o700);
        }
        Self {
            socket: parent.join("http.sock"),
            _temporary: temporary,
            root,
            parent,
            isolated,
        }
    }
}

pub fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}
