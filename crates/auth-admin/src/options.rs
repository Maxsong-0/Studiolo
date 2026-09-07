//! 命令参数只允许非秘密配置路径与显式迁移步骤，不回显未知输入。

use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Enroll,
    AuditFlush,
    Migrate(u8),
}

pub(crate) struct Options {
    pub(crate) command: Command,
    pub(crate) config: PathBuf,
}

pub(crate) fn parse(args: Vec<OsString>) -> Result<Option<Options>, crate::AdminError> {
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    if !matches!(args.len(), 3 | 5) || args[1] != "--config" {
        return Err(crate::AdminError::Arguments);
    }
    let command = match args[0].to_str() {
        Some("enroll") if args.len() == 3 => Command::Enroll,
        Some("audit-flush") if args.len() == 3 => Command::AuditFlush,
        Some("migrate") if args.len() == 5 && args[3] == "--from" => match args[4].to_str() {
            Some("1") => Command::Migrate(1),
            Some("2") => Command::Migrate(2),
            Some("3") => Command::Migrate(3),
            _ => return Err(crate::AdminError::Arguments),
        },
        _ => return Err(crate::AdminError::Arguments),
    };
    Ok(Some(Options {
        command,
        config: PathBuf::from(&args[2]),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_closed_commands_and_explicit_steps_are_accepted() {
        for name in ["enroll", "audit-flush"] {
            assert!(parse(args(&[name, "--config", "/private/config.json"])).is_ok());
        }
        for step in ["1", "2", "3"] {
            assert!(parse(args(&["migrate", "--config", "/x", "--from", step])).is_ok());
        }
        assert!(parse(args(&["--help"])).unwrap().is_none());
        for values in [
            vec![],
            vec!["enroll"],
            vec!["enroll", "--seed", "fake-secret"],
            vec!["recover", "--config", "/x"],
            vec!["enroll", "--config", "/x", "--stdin", "true"],
            vec!["migrate", "--config", "/x", "--from", "4"],
            vec!["migrate", "--config", "/x"],
        ] {
            assert!(parse(args(&values)).is_err());
        }
    }
}
