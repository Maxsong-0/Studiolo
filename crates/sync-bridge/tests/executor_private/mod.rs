//! B4 · MirrorCAS ＋ 双向 Bridge 执行器契约。
//! 假静止/搬运端口与内存审计只验证本核顺序与门禁，不代表真实 OwnedRun 或远端往返验收。

mod adapter;
mod behaviour;
mod cas_bounds;
mod failure;
mod fake_journal;
mod fuse;
mod ports;
mod recovery;
mod runner;
mod shutdown;
mod support;

mod drift;
