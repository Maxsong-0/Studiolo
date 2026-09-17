//! 两个预配置秘密槽的单向运行装配；权威绑定逐次检查，不缓存恢复成功或授权。

mod projection;

pub use projection::FactorState;

use super::{
    CsrfToken, PreparedTotpFactor, RecentAuthentication, Recovery, RecoveryCode, RecoveryCodes,
    SessionError, SessionInfo, SessionManager, SessionToken, Totp, TotpCode, TotpRequest, TotpSeed,
    denials::Denial, schema, state, totp,
};
use crate::AccessIdentity;
use rusqlite::Connection;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// 只提供提权、换码与预配置因子前滚；不登记、读取秘密环境或公开因子种子。
///
/// 种子须由可信装配事先安全保存并交付。两个槽的角色固定，替换槽激活后不把
/// 原槽变成恢复候选；重新配置下一组秘密仍须独立可信装配。
pub struct FactorRuntime {
    sessions: Arc<SessionManager>,
    current: Arc<totp::Context>,
    replacement: Option<Arc<totp::Context>>,
    current_enrollment: Option<i64>,
    prepared: Option<PreparedTotpFactor>,
    recovery: Recovery,
    replacement_seen: AtomicBool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Active {
    Current,
    Replacement,
}

struct Selected {
    active: Active,
    session: SessionInfo,
}

impl FactorRuntime {
    /// 为已登记 v4 装配固定当前槽及至多一个替换槽；不迁移、生成或登记种子。
    ///
    /// 按存储中的引用和秘密绑定选择活动槽，故成功替换或响应丢失后的重开可继续
    /// 使用已激活替换槽。拒绝未知绑定、同引用或不同引用下复用同一种子。
    /// 不证明外部秘密平面与 Auth 提交原子，也不抵御管理员一致回滚整库。
    pub fn attach(
        sessions: Arc<SessionManager>,
        configured_current: TotpSeed,
        configured_replacement: Option<TotpSeed>,
    ) -> Result<Self, SessionError> {
        if let Some(replacement) = &configured_replacement
            && (replacement.reference() == configured_current.reference()
                || replacement.matches_binding(
                    configured_current.reference(),
                    &configured_current.fingerprint(),
                ))
        {
            return Err(SessionError::Configuration);
        }
        let current = Arc::new(totp::Context {
            sessions: Arc::clone(&sessions),
            seed: configured_current,
        });
        let replacement = configured_replacement.map(|seed| {
            Arc::new(totp::Context {
                sessions: Arc::clone(&sessions),
                seed,
            })
        });
        let recovery = Recovery::attach(Arc::clone(&sessions))?;
        let (current_enrollment, prepared) = sessions.database.transaction(|db| {
            let factor = totp::store::factor(db)?.ok_or(SessionError::FactorUnavailable)?;
            let proof = factor
                .enrollment
                .as_ref()
                .ok_or(SessionError::FactorUnavailable)?;
            if replacement
                .as_ref()
                .is_some_and(|value| matches(value, &factor))
            {
                return Ok((None, None));
            }
            if !matches(&current, &factor) {
                return Err(SessionError::Configuration);
            }
            let prepared = replacement
                .as_ref()
                .map(|value| PreparedTotpFactor::from_context(Arc::clone(value), db))
                .transpose()?;
            Ok((Some(proof.operation.sequence), prepared))
        })?;
        Ok(Self {
            sessions,
            current,
            replacement,
            current_enrollment,
            prepared,
            recovery,
            replacement_seen: AtomicBool::new(current_enrollment.is_none()),
        })
    }

    /// 使用当前权威因子验证一次 OTP；原 TOTP 状态机仍重验凭据、预算和最终提交。
    pub fn step_up(
        &self,
        request: &TotpRequest<'_>,
        code: TotpCode,
    ) -> Result<RecentAuthentication, SessionError> {
        let active = self.select(request)?;
        let context = match active {
            Active::Current => &self.current,
            Active::Replacement => self
                .replacement
                .as_ref()
                .ok_or(SessionError::Configuration)?,
        };
        Totp {
            context: Arc::clone(context),
        }
        .step_up(request, code)
    }

    /// 以当前真实 L3 换发十枚恢复码，只交付原调用一次；不重读或自动重试原码。
    pub fn rotate_codes(&self, request: &TotpRequest<'_>) -> Result<RecoveryCodes, SessionError> {
        self.select(request)?;
        self.recovery.rotate_codes(request)
    }

    /// 可信维护补记 TOTP、恢复和最多 256 条会话事件，不重放动作或授予权限。
    ///
    /// 不依赖请求会话或重新选择秘密槽；替换后的旧句柄也只交付存储中的原事件。
    /// 返回本批新增 ACK 总数，不保证清空全部待办；失败前可能已有部分事件提交。
    pub fn flush_audit(&self) -> Result<usize, SessionError> {
        let totp = Totp {
            context: Arc::clone(&self.current),
        };
        let delivered = totp.flush_audit()? + self.recovery.flush_audit()?;
        Ok(delivered + self.sessions.flush_audit(256)?)
    }

    /// 只向预配置替换槽前滚；成功吊销全部旧会话，不返回新会话或通用 L3。
    ///
    /// 失败可能已消费输入或已提交切换。此方法从不自动重放；下一次调用重新选择
    /// 权威绑定，跨实例替换及提交后报错不会让内存中的旧句柄继续授权。
    pub fn recover(
        &self,
        request: &TotpRequest<'_>,
        code: RecoveryCode,
        otp: TotpCode,
    ) -> Result<(), SessionError> {
        if self.select(request)? != Active::Current {
            return Err(SessionError::FactorUnavailable);
        }
        let prepared = self
            .prepared
            .as_ref()
            .ok_or(SessionError::FactorUnavailable)?;
        self.recovery
            .replace_factor(request, code, prepared, otp)
            .map(|_| ())
    }

    fn select(&self, request: &TotpRequest<'_>) -> Result<Active, SessionError> {
        self.sessions.run(|db, observed| {
            Ok(self
                .select_in(db, observed, request.identity, request.token, request.csrf)?
                .map(|selected| selected.active))
        })
    }

    // 动作和显示投影共用同一选择规则；调用方必须在原run事务内使用本次结果。
    fn select_in(
        &self,
        db: &Connection,
        observed: &state::Observed,
        identity: &AccessIdentity,
        token: &SessionToken,
        csrf: &CsrfToken,
    ) -> Result<Result<Selected, SessionError>, SessionError> {
        if schema::version(db)? != 4 {
            return Ok(Err(SessionError::MigrationRequired));
        }
        let session = match self
            .sessions
            .authorized(db, observed, identity, token, Some(csrf))?
        {
            Ok(row) => row.info,
            Err(error) => return Ok(Err(error)),
        };
        let Some(factor) = totp::store::factor(db)? else {
            return state::deny(db, Denial::Issue, SessionError::FactorUnavailable);
        };
        let Some(proof) = &factor.enrollment else {
            return state::deny(db, Denial::Issue, SessionError::FactorUnavailable);
        };
        if self
            .replacement
            .as_ref()
            .is_some_and(|value| matches(value, &factor))
        {
            // 这里只收紧旧槽资格；即使随后的事务提交报错，也绝不退回旧槽。
            self.replacement_seen.store(true, Ordering::SeqCst);
            return Ok(Ok(Selected {
                active: Active::Replacement,
                session,
            }));
        }
        if !self.replacement_seen.load(Ordering::SeqCst)
            && self.current_enrollment == Some(proof.operation.sequence)
            && matches(&self.current, &factor)
        {
            return Ok(Ok(Selected {
                active: Active::Current,
                session,
            }));
        }
        state::deny(db, Denial::Issue, SessionError::Configuration)
    }
}

fn matches(context: &totp::Context, factor: &totp::model::Factor) -> bool {
    context.seed.reference() == factor.seed_ref
        && context
            .seed
            .matches_binding(&factor.seed_ref, &factor.seed_digest)
}
