//! 执行体人侧命令只走现有会话/CSRF；解冻与候选验收还需当前 L3。
use super::{AuthHttp, Context, cookie::Credentials, input::Action, responses::Failure};
use axum::{extract::Request, response::Response};
use serde::Deserialize;
use std::{net::IpAddr, sync::Arc};
use studiolo_auth::{AccessIdentity, FactorRuntime, SensitiveAction, SessionManager, TotpRequest};
use studiolo_core::TaskId;
use studiolo_task_engine::BoardReceipt;

pub(crate) const COMMAND: &str = "/api/v1/execution/command";
pub(crate) const STATUS: &str = "/api/v1/execution";
pub(crate) const JSON_LIMIT: usize = 131_072;
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Freeze {
        frozen: bool,
    },
    Start {
        task_id: TaskId,
        agent_type: String,
        instance_id: String,
    },
    Cancel {
        task_id: TaskId,
    },
    Land {
        task_id: TaskId,
        expected_head: String,
        patch_sha256: String,
    },
    Review {
        task_id: TaskId,
        receipt: BoardReceipt,
    },
}
pub(super) fn parse(bytes: &[u8]) -> Result<Input, Failure> {
    serde_json::from_slice(bytes).map_err(|_| Failure::bad_request())
}
pub(crate) struct Principal {
    sessions: Arc<SessionManager>,
    factors: Option<Arc<FactorRuntime>>,
    identity: AccessIdentity,
    credentials: Arc<Credentials>,
    source: Option<IpAddr>,
}
impl Principal {
    pub(crate) fn verify(&self) -> Result<(), String> {
        self.sessions
            .check_with_csrf(
                &self.identity,
                &self.credentials.token,
                &self.credentials.csrf,
            )
            .map(|_| ())
            .map_err(|_| "应用会话已失效".into())
    }
    pub(crate) fn sensitive(&self, action: SensitiveAction) -> Result<(), String> {
        let factors = self.factors.as_ref().ok_or("本入口未配置二次认证")?;
        let source = self.source.ok_or("需要可信Unix入口")?;
        factors
            .require_l3(
                &TotpRequest {
                    identity: &self.identity,
                    token: &self.credentials.token,
                    csrf: &self.credentials.csrf,
                    source,
                },
                action,
            )
            .map_err(|_| "请先完成二次认证（One-time Code）".to_owned())
    }
}
impl AuthHttp {
    pub(crate) fn execution(
        &self,
        request: &mut Request,
    ) -> Result<(Principal, Option<Input>), Box<Response>> {
        let context = request
            .extensions_mut()
            .remove::<Arc<Context>>()
            .ok_or_else(|| Failure::unauthorized().response())?;
        let context = Arc::try_unwrap(context).map_err(|_| Failure::unavailable().response())?;
        let input = match context.action {
            None => None,
            Some(Action::Execution(input)) => Some(input),
            _ => return Err(Box::new(Failure::bad_request().response())),
        };
        Ok((
            Principal {
                sessions: self.sessions.clone(),
                factors: self.factors.clone(),
                identity: context.identity,
                credentials: context
                    .credentials
                    .ok_or_else(|| Failure::unauthorized().response())?,
                source: context.source.as_ref().map(|s| s.ip()),
            },
            input,
        ))
    }
}
