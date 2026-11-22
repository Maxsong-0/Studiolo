//! 失败类别只是固定的几句话：互不相同，不带路径；交给作业的结果照旧只有两种。
use super::*;

const ALL: [CredentialFault; 9] = [
    CredentialFault::Account,
    CredentialFault::Parents,
    CredentialFault::Missing,
    CredentialFault::DirectoryAccess,
    CredentialFault::Storage,
    CredentialFault::FileAccess,
    CredentialFault::Content,
    CredentialFault::Changed,
    CredentialFault::Deadline,
];

#[test]
fn every_category_says_something_different_and_names_no_path() {
    let lines: Vec<String> = ALL.iter().map(ToString::to_string).collect();
    for (index, line) in lines.iter().enumerate() {
        assert!(line.starts_with("取不到备份凭据（"), "{line}");
        assert!(line.contains(&format!("{:?}", ALL[index])), "{line}");
        assert!(!line.contains('/'), "{line}");
        assert!(lines.iter().filter(|other| *other == line).count() == 1);
    }
}

#[test]
fn what_jobs_and_checks_are_told_stays_the_same_two_reasons() {
    for fault in ALL {
        let expected = if fault == CredentialFault::Deadline {
            ProcessError::Deadline
        } else {
            ProcessError::Configuration
        };
        assert_eq!(ProcessError::from(fault), expected, "{fault:?}");
    }
}
