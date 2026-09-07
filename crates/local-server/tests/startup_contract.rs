//! 真实产品进程启动契约：临时双库、虚构身份，不访问生产材料或替换 JWKS。

#![cfg(unix)]

#[path = "startup/cases.rs"]
mod cases;
#[path = "startup/factors.rs"]
mod factors;
#[path = "startup/fixture.rs"]
mod fixture;
#[path = "startup/legacy.rs"]
mod legacy;
#[path = "startup/paths.rs"]
mod paths;
#[path = "startup/process.rs"]
mod process;
#[path = "startup/writer.rs"]
mod writer;
