//! 共用元数据与流式事实内核；没有写能力、授权回调或宿主绝对路径。
use super::{SyncReadSelection, metadata, state::*};
use crate::{BrokerError, VaultPath, VaultUuid, blob::observed};
use cap_std::fs::Dir;
use std::collections::BTreeSet;

pub(crate) fn begin(
    root: &Dir,
    private: bool,
    selection: &SyncReadSelection,
    required: &[VaultPath],
) -> Result<Survey, BrokerError> {
    let index = selection.validate(required)?;
    let survey = metadata::capture(root, private, selection, required, index)?;
    #[cfg(test)]
    super::tests::hook(0);
    budget(&survey)?;
    check_survey(root, private, &survey)?;
    Ok(survey)
}

pub(crate) fn finish(
    root: &Dir,
    private: bool,
    mut survey: Survey,
    additional: &[VaultPath],
) -> Result<Prepared, BrokerError> {
    if additional.len() > 4096 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    // 额外输入自身不得重复；与初始路径重合允许，便于两侧直接交完整并集。
    let mut seen = BTreeSet::new();
    for path in additional {
        if !seen.insert(path) {
            return Err(super::invalid());
        }
    }
    for path in survey.required.iter().chain(additional) {
        survey.index.add(path)?;
        if survey.selection.file_limit(path).is_none() {
            return Err(super::invalid());
        }
        survey.paths.insert(path.clone());
        if survey.paths.len() > 4096 {
            return Err(BrokerError::WriterLimitExceeded);
        }
    }
    check_survey(root, private, &survey)?;
    for path in &survey.paths {
        if survey.candidates.contains_key(path) {
            continue;
        }
        // 新路径只能通过首次完整名字和固定父链证明缺失，不能重扫或补猜测。
        let (parents, absent) = metadata::parent_chain(path, &survey.directories)?;
        if !absent {
            return Err(super::invalid());
        }
        observed::missing(root, path, &parents, private)?;
        survey
            .candidates
            .insert(path.clone(), Candidate::Missing(parents));
    }
    budget(&survey)?;
    check_survey(root, private, &survey)?;
    Ok(Prepared {
        selection: survey.selection,
        candidates: survey.candidates,
        proofs: Proofs {
            directories: survey
                .directories
                .into_iter()
                .map(|(p, d)| (p, d.stamp))
                .collect(),
            files: Default::default(),
        },
    })
}
fn budget(survey: &Survey) -> Result<(), BrokerError> {
    let mut total = 0_u64;
    for (path, candidate) in &survey.candidates {
        let limit = survey
            .selection
            .file_limit(path)
            .ok_or_else(super::invalid)?;
        if let Candidate::Present(node) = candidate {
            if node.file.bytes > limit {
                return Err(BrokerError::WriterLimitExceeded);
            }
            total = total
                .checked_add(node.file.bytes)
                .ok_or(BrokerError::WriterLimitExceeded)?;
            if total > 4 * 1024 * 1024 * 1024 {
                return Err(BrokerError::WriterLimitExceeded);
            }
        }
    }
    Ok(())
}
fn check_survey(root: &Dir, private: bool, survey: &Survey) -> Result<(), BrokerError> {
    for (path, directory) in &survey.directories {
        if observed::directory_stamp(root, path, private)? != directory.stamp {
            return Err(BrokerError::CaptureChanged);
        }
    }
    for (path, candidate) in &survey.candidates {
        match candidate {
            Candidate::Present(node) => observed::check(root, path, node, private)?,
            Candidate::Missing(parents) => observed::missing(root, path, parents, private)?,
        }
    }
    Ok(())
}
pub(crate) fn read(
    root: &Dir,
    private: bool,
    path: &VaultPath,
    candidate: Candidate,
    selection: &SyncReadSelection,
    vault: Option<VaultUuid>,
) -> Result<(Option<Fact>, Proof), BrokerError> {
    let limit = selection.file_limit(path).ok_or_else(super::invalid)?;
    match candidate {
        Candidate::Missing(parents) => {
            observed::missing(root, path, &parents, private)?;
            Ok((None, Proof::Missing(parents)))
        }
        Candidate::Present(node) => {
            #[cfg(test)]
            super::tests::hook(1);
            let (proof, utf8_valid) = observed::facts(root, path, &node, limit, vault, private)?;
            let fact = Fact {
                identity: proof.identity.clone(),
                revision: proof.revision,
                utf8_valid,
            };
            Ok((Some(fact), Proof::Present(proof)))
        }
    }
}
pub(crate) fn revalidate(root: &Dir, private: bool, proofs: &Proofs) -> Result<(), BrokerError> {
    for (path, stamp) in &proofs.directories {
        if observed::directory_stamp(root, path, private)? != *stamp {
            return Err(BrokerError::CaptureChanged);
        }
    }
    for (path, proof) in &proofs.files {
        match proof {
            Proof::Present(proof) => observed::check(root, path, &proof.node, private)?,
            Proof::Missing(parents) => observed::missing(root, path, parents, private)?,
        }
    }
    Ok(())
}
