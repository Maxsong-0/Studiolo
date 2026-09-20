//! 命令参数只允许非秘密配置路径与显式迁移步骤，不回显未知输入。

use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Enroll,
    AuditFlush,
    Migrate(u8),
    RestoreInvalidate,
}

pub(crate) struct Options {
    pub(crate) command: Command,
    pub(crate) config: PathBuf,
    pub(crate) restore: Option<RestoreOptions>,
}

pub(crate) struct RestoreOptions {
    pub(crate) root: PathBuf,
    pub(crate) id: String,
}

pub(crate) fn parse(args: Vec<OsString>) -> Result<Option<Options>, crate::AdminError> {
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    if args.first().is_some_and(|arg| arg == "restore-invalidate") {
        return parse_restore(&args).map(Some);
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
        restore: None,
    }))
}

fn parse_restore(args: &[OsString]) -> Result<Options, crate::AdminError> {
    if args.len() != 7
        || args[1] != "--config"
        || args[3] != "--restore-root"
        || args[5] != "--restore-id"
    {
        return Err(crate::AdminError::Arguments);
    }
    let id = args[6].to_str().ok_or(crate::AdminError::Arguments)?;
    if !identifier(id, 55) {
        return Err(crate::AdminError::Arguments);
    }
    let config = PathBuf::from(&args[2]);
    let root = PathBuf::from(&args[4]);
    if !config.is_absolute() || !root.is_absolute() {
        return Err(crate::AdminError::Arguments);
    }
    Ok(Options {
        command: Command::RestoreInvalidate,
        config,
        restore: Some(RestoreOptions {
            root,
            id: id.into(),
        }),
    })
}

pub(crate) fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
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

    #[test]
    fn restore_arguments_are_closed_bounded_and_never_echoed() {
        let base = [
            "restore-invalidate",
            "--config",
            "/x/owner.json",
            "--restore-root",
            "/x",
            "--restore-id",
            "run-1",
        ];
        let parsed = parse(args(&base)).unwrap().unwrap();
        assert_eq!(parsed.command, Command::RestoreInvalidate);
        assert_eq!(parsed.restore.unwrap().id, "run-1");
        for id in ["", "a_b", "../x", "秘密", &"a".repeat(56)] {
            let mut values = args(&base);
            values[6] = id.into();
            assert!(matches!(parse(values), Err(crate::AdminError::Arguments)));
        }
        for (index, value) in [
            (1, "--seed"),
            (3, "--config"),
            (5, "--unknown"),
            (2, "relative"),
            (4, "relative"),
        ] {
            let mut values = args(&base);
            values[index] = value.into();
            assert!(parse(values).is_err());
        }
        for values in [
            args(&base[..6]),
            [args(&base), args(&["fictional-secret"])].concat(),
        ] {
            let error = parse(values).err().unwrap();
            assert!(!format!("{error:?} {error}").contains("fictional-secret"));
        }
    }
}
