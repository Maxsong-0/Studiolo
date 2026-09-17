//! 每次正文操作把原始身份链送入唯一 owner 后重验，不缓存授权结果。

use super::{AuthHttp, Context, cookie::Credentials, input::Action, responses::Failure};
use axum::{extract::Request, response::Response};
use std::sync::Arc;
use studiolo_auth::{AccessIdentity, SessionError, SessionInfo, SessionManager};

pub(crate) struct ContentPrincipal {
    sessions: Arc<SessionManager>,
    identity: AccessIdentity,
    credentials: Arc<Credentials>,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl ContentPrincipal {
    pub(crate) fn verify(&self) -> Result<SessionInfo, SessionError> {
        self.sessions.check_with_csrf(
            &self.identity,
            &self.credentials.token,
            &self.credentials.csrf,
        )
    }
    pub(crate) fn actor(&self) -> (&str, &str) {
        (self.identity.issuer(), self.identity.subject())
    }
}

impl AuthHttp {
    pub(crate) fn file_operation(
        &self,
        request: &mut Request,
    ) -> Result<(ContentPrincipal, crate::file_operations::Input), Box<Response>> {
        let context = request
            .extensions_mut()
            .remove::<Arc<Context>>()
            .ok_or_else(|| Failure::unauthorized().response())?;
        let context = Arc::try_unwrap(context).map_err(|_| Failure::unavailable().response())?;
        let Some(Action::FileOperation(input)) = context.action else {
            return Err(Box::new(Failure::bad_request().response()));
        };
        Ok((
            ContentPrincipal {
                sessions: self.sessions.clone(),
                identity: context.identity,
                credentials: context
                    .credentials
                    .ok_or_else(|| Failure::unauthorized().response())?,
                _permit: context.content_permit,
            },
            input,
        ))
    }
    pub(crate) fn content_reader(
        &self,
        request: &mut Request,
    ) -> Result<ContentPrincipal, Box<Response>> {
        let context = request
            .extensions_mut()
            .remove::<Arc<Context>>()
            .ok_or_else(|| Failure::unauthorized().response())?;
        let context = Arc::try_unwrap(context).map_err(|_| Failure::unavailable().response())?;
        if context.action.is_some() {
            return Err(Box::new(Failure::bad_request().response()));
        }
        Ok(ContentPrincipal {
            sessions: self.sessions.clone(),
            identity: context.identity,
            credentials: context
                .credentials
                .ok_or_else(|| Failure::unauthorized().response())?,
            _permit: context.content_permit,
        })
    }

    pub(crate) fn content(
        &self,
        request: &mut Request,
    ) -> Result<(ContentPrincipal, crate::editing::Replace), Box<Response>> {
        let context = request
            .extensions_mut()
            .remove::<Arc<Context>>()
            .ok_or_else(|| Failure::unauthorized().response())?;
        let context = Arc::try_unwrap(context).map_err(|_| Failure::unavailable().response())?;
        let Some(Action::Content(input)) = context.action else {
            return Err(Box::new(Failure::bad_request().response()));
        };
        Ok((
            ContentPrincipal {
                sessions: self.sessions.clone(),
                identity: context.identity,
                _permit: context.content_permit,
                credentials: context
                    .credentials
                    .ok_or_else(|| Failure::unauthorized().response())?,
            },
            input,
        ))
    }
}
