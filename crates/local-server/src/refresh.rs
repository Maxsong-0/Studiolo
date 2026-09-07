//! 旧只读刷新继续串行等待；实际闭包持许可并负责发布，不由 HTTP future 代持。

use axum::http::StatusCode;
use std::sync::Arc;

use crate::{ApiError, AppState, LocalError, Source, load};

pub(crate) async fn run(state: &AppState) -> Result<(), ApiError> {
    match &state.source {
        Source::Writer(workbench) => workbench
            .refresh()
            .await
            .map_err(|_| ApiError::unavailable()),
        Source::Replica(vault) => {
            let permit = state.refresh.clone().lock_owned().await;
            let vault = Arc::clone(vault);
            let view = state.current.clone();
            #[cfg(test)]
            let probe = Arc::clone(&state.refresh_probe);
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                #[cfg(test)]
                probe.enter();
                let next = view
                    .snapshot()?
                    .generation
                    .number
                    .checked_add(1)
                    .ok_or(LocalError::Unavailable)?;
                let replacement = load(&vault, next)?;
                let previous = view
                    .0
                    .write()
                    .map_err(|_| LocalError::Unavailable)?
                    .generation
                    .replace(Arc::new(replacement));
                drop(previous);
                Ok::<_, LocalError>(())
            })
            .await
            .map_err(|_| ApiError::unavailable())?
            .map_err(|error| ApiError(StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}
