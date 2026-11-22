//! 三槽来自同一原根；读取后和每次执行前后都核验原文件与原路径。
use super::{
    fault::Result,
    file::{SecretFile, Slot},
    root::Root,
};
use std::time::Instant;
use zeroize::Zeroizing;

pub(super) struct Input {
    root: Root,
    files: [SecretFile; 3],
}
pub(super) struct Values {
    pub input: Input,
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
}
impl Input {
    pub fn load(uid: u32, deadline: Instant) -> Result<Values> {
        let root = Root::open_credentials(uid, deadline)?;
        let (repository, repository_value) = SecretFile::load(&root, Slot::Repository, deadline)?;
        // repository 密码仍由 restic 读固定只读文件；父进程只检验格式，不保留正文。
        drop(repository_value);
        let (username_file, username) = SecretFile::load(&root, Slot::Username, deadline)?;
        let (password_file, password) = SecretFile::load(&root, Slot::Password, deadline)?;
        let input = Self {
            root,
            files: [repository, username_file, password_file],
        };
        input.check(deadline)?;
        Ok(Values {
            input,
            username,
            password,
        })
    }
    pub fn check(&self, deadline: Instant) -> Result<()> {
        self.root.check(deadline)?;
        for file in &self.files {
            file.check(&self.root, deadline)?;
        }
        self.root.check(deadline)
    }
}
