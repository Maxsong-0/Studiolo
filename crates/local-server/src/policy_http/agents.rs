//! 接入方列表：身份与策略各取一个完整快照，仅投影主人可见的封闭字段。
use super::{
    PolicyPlane,
    common::{now, plane, principal, respond, run},
    problem::Problem,
};
use crate::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use studiolo_policy::{Denial, RevocationRecord};
const LIMIT: usize = 500;
#[derive(Serialize)]
struct TypeRow {
    name: String,
    issued_at: String,
    identity_revoked_at: Option<String>,
    capability_version: Option<u64>,
    policy_revocation: Option<RevocationRecord>,
}
#[derive(Serialize)]
struct Refusal {
    reason: &'static str,
    by: Option<String>,
}
#[derive(Serialize)]
struct InstanceRow {
    id: String,
    agent_type: String,
    parent: Option<String>,
    registered_at: String,
    last_seen_at: String,
    denial: Option<Refusal>,
    policy_revocation: Option<RevocationRecord>,
}
#[derive(Serialize)]
struct Body {
    generated_at: String,
    effective_hash: String,
    global_revoked: bool,
    children_enabled: bool,
    types: Vec<TypeRow>,
    instances: Vec<InstanceRow>,
    instances_truncated: bool,
}
fn unavailable() -> Problem {
    Problem::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "agents_unavailable",
        "接入方身份或策略暂不可用，请稍后重试。",
    )
}
pub(super) async fn list(State(state): State<AppState>, mut request: Request) -> Response {
    let plane = match plane(&request) {
        Ok(value) => value,
        Err(p) => return p.into_response(),
    };
    let (principal, _) = match principal(&state, &mut request) {
        Ok(value) => value,
        Err(r) => return *r,
    };
    respond(run(plane, principal, |plane, _| read(plane)).await)
}
fn read(plane: &PolicyPlane) -> Result<Body, Problem> {
    let view = plane.identities.as_ref().ok_or_else(|| {
        Problem::new(
            StatusCode::NOT_FOUND,
            "agents_plane_absent",
            "本入口未装配接入方列表。",
        )
    })?;
    let mut identity = view.snapshot(LIMIT + 1).map_err(|_| unavailable())?;
    let instances_truncated = identity.instances.len() > LIMIT;
    identity.instances.truncate(LIMIT);
    let names = identity
        .types
        .iter()
        .map(|r| r.name.clone())
        .collect::<Vec<_>>();
    let principals = identity
        .instances
        .iter()
        .flat_map(|row| &identity.lineages[&row.instance.id])
        .map(|i| (i.id.clone(), i.agent_type.clone()))
        .collect();
    let mut policy = plane
        .store
        .agent_snapshot(&names, &principals)
        .map_err(|_| unavailable())?;
    let mut instances = Vec::with_capacity(identity.instances.len());
    for row in identity.instances {
        let current = policy
            .instances
            .get(&row.instance.id)
            .ok_or_else(unavailable)?;
        let kind = identity
            .types
            .iter()
            .find(|t| t.name == row.instance.agent_type)
            .ok_or_else(unavailable)?;
        let denial = if kind.revoked_at.is_some() {
            Some(Refusal {
                reason: "identity_type_revoked",
                by: None,
            })
        } else if let Some(reason) = current.denial {
            Some(Refusal {
                reason: reason.as_str(),
                by: None,
            })
        } else {
            identity.lineages[&row.instance.id]
                .iter()
                .skip(1)
                .find(|ancestor| {
                    policy
                        .instances
                        .get(&ancestor.id)
                        .is_some_and(|state| state.denial == Some(Denial::InstanceRevoked))
                })
                .map(|ancestor| Refusal {
                    reason: "ancestor_revoked",
                    by: Some(ancestor.id.clone()),
                })
        };
        instances.push(InstanceRow {
            id: row.instance.id,
            agent_type: row.instance.agent_type,
            parent: row.parent,
            registered_at: row.instance.registered_at,
            last_seen_at: row.last_seen_at,
            denial,
            policy_revocation: current.revocation.clone(),
        });
    }
    let types = identity
        .types
        .into_iter()
        .map(|row| {
            let state = policy.types.remove(&row.name).ok_or_else(unavailable)?;
            Ok(TypeRow {
                name: row.name,
                issued_at: row.issued_at,
                identity_revoked_at: row.revoked_at,
                capability_version: state.capability_version,
                policy_revocation: state.revocation,
            })
        })
        .collect::<Result<_, Problem>>()?;
    Ok(Body {
        generated_at: now(),
        effective_hash: policy.effective_hash,
        global_revoked: policy.global_revoked,
        children_enabled: identity.children_enabled,
        types,
        instances,
        instances_truncated,
    })
}
