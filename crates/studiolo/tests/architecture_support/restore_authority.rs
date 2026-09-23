//! 离线恢复只在其固定的私有布局内钉住只读描述符。
use super::authority_tokens::{checked_closed, compact};

pub(super) fn remainder(source: &str) -> String {
    let normalized = compact(source);
    if normalized.matches("openat(").count() != 2 || normalized.matches("File::from(").count() != 3
    {
        return normalized;
    }
    checked_closed(
        source,
        &[
            "pub(crate)constBASE:&str=\"/var/lib/studiolo-dr-tests\";",
            "constFLAGS:OFlags=OFlags::RDONLY.union(OFlags::NOFOLLOW).union(OFlags::NONBLOCK).union(OFlags::CLOEXEC);",
            "if!linux||real!=0||effective!=0||tty||!separate",
            "leaf!=\"human-work\"",
            "config.as_os_str()!=root.join(\"owner.json\").as_os_str()",
            "layout(root,config)?;",
            "context(true,0,0,false,identity(&own)!=identity(&initial))?;",
            "letflags=FLAGS|ifmatches!(kind,Kind::Ancestor|Kind::Directory){OFlags::DIRECTORY}else{OFlags::empty()};",
            "guard.revalidate()?;",
        ],
        &[
            ("rustix::fs::{Mode,OFlags,open,openat}", 1),
            ("fs::{self,File,Metadata}", 1),
            ("os::unix::fs::MetadataExt", 1),
            ("File::open(\"/proc/self/ns/net\")", 2),
            ("File::open(\"/proc/1/ns/net\")", 1),
            ("fs::symlink_metadata(&self.path)", 1),
            ("fs::symlink_metadata(entry.path())", 1),
            ("fs::read_dir(path)", 1),
            (
                "File::from(open(\"/\",FLAGS|OFlags::DIRECTORY,Mode::empty()).map_err(|_|AdminError::RestoreGate)?)",
                1,
            ),
            (
                "File::from(openat(&self.pins[parent].file,name,flags,Mode::empty()).map_err(|_|AdminError::RestoreGate)?)",
                1,
            ),
            (
                "File::from(openat(&pinned.file,&name,FLAGS,Mode::empty()).map_err(|_|AdminError::RestoreGate)?)",
                1,
            ),
        ],
    )
}
