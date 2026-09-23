//! 项目回放把有界的权威事件分页与新鲜的带日期日志指针合并。
use super::{
    BoardPlane,
    http::{busy, unavailable, unconfigured},
};
use axum::{
    Extension, Json,
    extract::{Query, rejection::QueryRejection},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc};
use studiolo_audit_ledger::AuditFilter;
use studiolo_core::VaultPath;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TimelineQuery {
    project: String,
    from: String,
    to: String,
    after: Option<u64>,
}

pub(super) async fn timeline(
    plane: Option<Extension<Arc<BoardPlane>>>,
    query: Result<Query<TimelineQuery>, QueryRejection>,
) -> Response {
    // 反序列化失败、时间戳不可解析、项目或区间越界都是畸形查询串：同一固定中文 JSON。
    let invalid = || crate::invalid_query("invalid_timeline_query");
    // 查询串拒绝仍先于平面检查，状态码与原提取器一致。
    let Ok(Query(query)) = query else {
        return invalid();
    };
    let Some(Extension(plane)) = plane else {
        return unconfigured();
    };
    let Ok(from) = query.from.parse::<jiff::Timestamp>() else {
        return invalid();
    };
    let Ok(to) = query.to.parse::<jiff::Timestamp>() else {
        return invalid();
    };
    if VaultPath::parse(&query.project).is_err()
        || !query.project.starts_with("项目/")
        || query.project.split('/').count() != 2
        || from > to
        || to.as_second().saturating_sub(from.as_second()) > 31 * 86400
    {
        return invalid();
    }
    let permit = match plane.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return busy(),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        plane.storage.check().map_err(str::to_owned)?;
        let hash = plane
            .policy
            .effective()
            .map_err(|_| "策略不可用")?
            .effective_hash;
        let engine = plane.engine.lock().map_err(|_| "黑板不可用")?;
        let tasks = engine
            .board_list(&query.project, &hash)
            .map_err(|e| e.to_string())?;
        let ids = tasks
            .iter()
            .map(|t| t.task.task_id)
            .collect::<BTreeSet<_>>();
        // 每页至多扫描 1001 条全局元数据记录。即使跨过的是无关项目，游标也照样前进；
        // 游标前进从不断言被跨过范围内的事件不存在。
        let records = plane
            .audit
            .query(
                &AuditFilter::default()
                    .after_offset(query.after.unwrap_or(0))
                    .with_limit(1001),
            )
            .map_err(|_| "审计不可用")?;
        let events = records
            .iter()
            .take(1000)
            .filter(|r| {
                r.recorded_at() >= from
                    && r.recorded_at() <= to
                    && r.event()
                        .subject()
                        .task_id()
                        .is_some_and(|id| ids.contains(&id))
            })
            .map(|r| {
                json!({
                    "offset":r.offset().to_string(), "at":r.recorded_at().to_string(),
                    "task_id":r.event().subject().task_id(), "action":r.event().action(),
                    "phase":r.event().phase(), "attempt_id":r.event().subject().attempt_id(),
                })
            })
            .collect::<Vec<_>>();
        let next = (records.len() > 1000).then(|| records[999].offset().to_string());
        let workbench = &engine.broker().workbench;
        workbench
            .revalidate()
            .map_err(|_| "日志索引正在恢复，请稍后重试")?;
        let view = workbench
            .core
            .view
            .snapshot()
            .map_err(|_| "日志索引不可用")?;
        if view.freshness.as_ref().is_none_or(|f| f.stale) {
            return Err("日志索引不是新鲜权威状态".into());
        }
        let prefix = format!("{}/02-日志/", query.project);
        let start = from.to_string()[..10].to_owned();
        let end = to.to_string()[..10].to_owned();
        let mut logs = Vec::new();
        let mut undated = 0;
        for document in view.generation.model.snapshot().documents {
            let Some(name) = document.path.strip_prefix(&prefix) else {
                continue;
            };
            let Some(date) = name
                .get(..10)
                .filter(|s| s.parse::<jiff::civil::Date>().is_ok())
            else {
                undated += 1;
                continue;
            };
            if date >= start.as_str() && date <= end.as_str() {
                if logs.len() >= 366 {
                    return Err("日志数量超过单次回放上限，请缩短范围".into());
                }
                logs.push(json!({"path":document.path,"title":document.title,"date":date}));
            }
        }
        workbench
            .revalidate()
            .map_err(|_| "回放期间日志索引发生变化")?;
        let current = workbench
            .core
            .view
            .snapshot()
            .map_err(|_| "日志索引不可用")?;
        if current.generation.number != view.generation.number {
            return Err("回放期间日志代次变化".into());
        }
        Ok::<_, String>(
            json!({"project":query.project,"from":query.from,"to":query.to,
            "events":events,"logs":logs,"undated_logs":undated,"next":next}),
        )
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => unavailable(),
    }
}
