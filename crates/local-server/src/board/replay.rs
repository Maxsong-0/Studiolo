//! 图3只读回放只提供权威元数据，原始模型响应与提示词永不进入监督JSON。
use super::BoardPlane;
use axum::{
    Extension, Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use studiolo_audit_ledger::AuditFilter;
use studiolo_core::TaskId;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplayQuery {
    task_id: TaskId,
    after: Option<u64>,
}
pub(super) async fn replay(
    plane: Option<Extension<Arc<BoardPlane>>>,
    Query(query): Query<ReplayQuery>,
) -> Response {
    let Some(Extension(plane)) = plane else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let permit = match plane.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
    };
    let result=tokio::task::spawn_blocking(move||{
        let _permit=permit;plane.storage.check().map_err(str::to_owned)?;
        let hash=plane.policy.effective().map_err(|_|"策略不可用")?.effective_hash;
        let engine=plane.engine.lock().map_err(|_|"黑板不可用")?;
        let task=engine.board_detail(query.task_id,&hash).map_err(|e|e.to_string())?.ok_or("工作项不存在")?;
        let filter=AuditFilter::default().with_task(query.task_id).after_offset(query.after.unwrap_or(0)).with_limit(101);
        let records=plane.audit.query(&filter).map_err(|_|"权威审计不可用")?;
        let more=records.len()>100;
        let events=records.iter().take(100).map(|record|{
            let event=record.event();let source=event.payload().as_value();let mut details=serde_json::Map::new();
            for key in ["state","status","fence","call_id","provider","model","snapshot_id","policy_hash","decision","reason","receipt_sha256","manifest_sha256"]{
                if let Some(value)=source.get(key){details.insert(key.into(),value.clone());}
            }
            json!({"offset":record.offset().to_string(),"at":record.recorded_at().to_string(),"action":event.action(),"phase":event.phase(),"attempt_id":event.subject().attempt_id(),"details":details})
        }).collect::<Vec<_>>();
        let next=if more{records.get(99).map(|r|r.offset().to_string())}else{None};
        Ok::<_,String>(json!({"task":task,"events":events,"next":next}))
    }).await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
