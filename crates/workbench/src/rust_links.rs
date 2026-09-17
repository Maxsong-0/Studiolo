//! 只在最近允许范围内关联唯一自由函数；不透明或遮蔽时不附候选目标。

use tree_sitter::Tree;

use crate::{
    RustCallAssociation, RustCallSite, RustSourceError, RustSourceInput,
    rust_scopes::{self, Call, Index},
};

pub(crate) fn derive(
    tree: &Tree,
    input: &RustSourceInput,
    file: usize,
    calls: &[RustCallSite],
) -> Result<Vec<RustCallAssociation>, RustSourceError> {
    let index = rust_scopes::build(tree, input, file)?;
    if index.calls.len() != calls.len() {
        return Err(RustSourceError::invalid(
            "Rust 关联与同批调用位置数量不一致。",
        ));
    }
    calls
        .iter()
        .map(|site| {
            let call = index
                .calls
                .get(&site.id)
                .ok_or_else(|| RustSourceError::invalid("Rust 关联缺少同批调用语法位置。"))?;
            let (target, reason) = match resolve(&index, call) {
                Ok(target) => (Some(target.to_owned()), None),
                Err(reason) => (None, Some(reason.to_owned())),
            };
            Ok(RustCallAssociation {
                call_id: site.id.clone(),
                status: if target.is_some() {
                    "linked"
                } else {
                    "unresolved"
                }
                .to_owned(),
                target,
                reason,
            })
        })
        .collect()
}

fn resolve<'a>(index: &'a Index, call: &Call) -> Result<&'a str, &'static str> {
    let key = call.key.as_ref().ok_or("callee_form")?;
    let scope = &index.scopes[call.scope];
    if !scope.function || scope.forbidden {
        return Err("context");
    }
    let mut next = Some(call.scope);
    while let Some(id) = next {
        let scope = &index.scopes[id];
        if scope.opaque {
            return Err("opaque_scope");
        }
        next = scope.parent;
    }
    let mut next = Some(call.scope);
    while let Some(id) = next {
        let scope = &index.scopes[id];
        if let Some(name) = scope.names.get(key) {
            if name.blocker {
                return Err("shadowed_name");
            }
            if name.candidates.len() > 1 {
                return Err("ambiguous_name");
            }
            if let Some(target) = name.candidates.first() {
                if target.generic {
                    return Err("generic_target");
                }
                return Ok(&target.target);
            }
        }
        if scope.boundary {
            return Err("not_found");
        }
        next = scope.parent;
    }
    Err("not_found")
}
