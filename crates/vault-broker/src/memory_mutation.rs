//! 内存替身沿用磁盘请求指纹与 CAS 拒绝终态，后续 ABA 不会复活旧请求。

use crate::memory_namespace;
use crate::mutation::record::{Outcome, cas_matches};
use crate::{BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation, MutationReceipt, Revision};

pub(crate) fn apply(
    broker: &mut MemoryVaultBroker,
    mutation: Mutation,
) -> Result<MutationReceipt, BrokerError> {
    crate::mutation::validate_request(&mutation)?;
    let fingerprint = FsVaultBroker::fingerprint(&mutation)?;
    if let Some((prior, outcome)) = broker.replays.get(&mutation.idempotency_key) {
        if prior != &fingerprint {
            return Err(BrokerError::IdempotencyConflict);
        }
        return outcome.clone().result();
    }
    // 与磁盘同序：重放命中之后、CAS 之前观察命名空间，对应 `blob::current::observe`。
    memory_namespace::guard_write(broker, &mutation.path)?;
    let current = broker.objects.get(&mutation.path);
    let actual = current.map(|bytes| Revision::compute(broker.vault_uuid, &mutation.path, bytes));
    if !cas_matches(mutation.precondition, actual) {
        let outcome = Outcome::Rejected {
            path: mutation.path,
            precondition: mutation.precondition,
            actual,
        };
        broker
            .replays
            .insert(mutation.idempotency_key, (fingerprint, outcome.clone()));
        return outcome.result();
    }
    if let (Some(bytes), Some(revision)) = (current, actual) {
        broker
            .history
            .entry(revision)
            .or_insert_with(|| bytes.clone());
    }
    let revision = Revision::compute(broker.vault_uuid, &mutation.path, &mutation.content);
    broker
        .objects
        .insert(mutation.path.clone(), mutation.content);
    let receipt = MutationReceipt {
        path: mutation.path,
        revision,
        previous_revision: actual,
    };
    broker.replays.insert(
        mutation.idempotency_key,
        (
            fingerprint,
            Outcome::Committed {
                receipt: receipt.clone(),
            },
        ),
    );
    Ok(receipt)
}
