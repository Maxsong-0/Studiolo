use super::super::inputs::Inputs;
use studiolo_core::VaultPath;
use studiolo_vault_broker::{
    FileOperationLimits, FileOperationMetadata, FileOperationRequest, FileOperationScopeHash,
    MutationActor,
};

pub(super) fn initialize(inputs: &mut Inputs) {
    inputs
        .broker
        .enable_file_operations(FileOperationLimits {
            effective_object_limit: 4096,
            max_tombstones: 64,
            // Broker 预留完整未知 current 证据容量；实际夹具仍只有几十字节。
            retained_bytes_limit: 2 * 1024 * 1024 * 1024,
            minimum_free_bytes: 1024,
        })
        .unwrap();
}

pub(super) fn metadata() -> FileOperationMetadata {
    FileOperationMetadata::new(
        MutationActor::Human {
            issuer: "https://original.test".into(),
            subject: "original-human".into(),
        },
        FileOperationScopeHash::new("1".repeat(64)).unwrap(),
        1,
    )
    .unwrap()
}

pub(super) fn request(inputs: &Inputs) -> FileOperationRequest {
    let path = VaultPath::parse("note.md").unwrap();
    let original = inputs.broker.read_file(&path, 4096).unwrap();
    FileOperationRequest::Delete {
        path,
        expected: original.revision,
        idempotency_key: "PRIVATE_TYPED_REQUEST_KEY".into(),
    }
}

pub(super) fn lost_delete(inputs: &mut Inputs) -> FileOperationRequest {
    initialize(inputs);
    let request = request(inputs);
    drop(
        inputs
            .broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap(),
    );
    request
}

pub(super) fn selector(inputs: &Inputs) -> String {
    inputs.broker.maintenance_inspect(64).unwrap().targets[0]
        .review_id
        .clone()
}
