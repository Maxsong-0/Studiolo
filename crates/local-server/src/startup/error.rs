//! 启动边界只返回固定脱敏说明，不回显根路径、配置或底层监视错误。

use studiolo_auth::SessionError;
use studiolo_local_server::LocalError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum StartupError {
    #[error("启动配置或目录隔离检查失败；仅接受明确的非秘密私有配置")]
    Configuration,
    #[error("既有审计库不可用；不创建、不修复、不回退")]
    Audit,
    #[error("{0}")]
    Auth(#[from] SessionError),
    #[error("只读根不可用或已被替换；未监听")]
    ReadRoots,
    #[error("Access公钥初始化失败；未监听，不回退为本机模式")]
    Access,
    /// 只带固定中文类别：库内路径、诊断正文与底层英文错误都不进启动错误。
    #[error("只读路由初始化失败：{reason}；未监听")]
    Router { reason: &'static str },
    #[error("无法绑定指定loopback端口")]
    Listen,
    #[error("HTTP服务已停止：监听处理失败")]
    Serve,
    #[error("因子秘密环境缺失或不规范；只接受固定槽的40位小写十六进制")]
    FactorEnvironment,
    /// 本平台声明支持的收紧没能生效；与是否启用因子模式无关，一律不开始服务。
    #[error("{0}")]
    Residency(#[from] studiolo_process_safety::Refusal),
    /// 逐项分类必须被操作者看见；记录不下来就等于静默跳过，同样不开始服务。
    #[error("无法记录秘密驻留面逐项分类；未开始服务")]
    ResidencyReport,
    #[error("当前进程保护或退出信号设置失败；未开始服务")]
    Process,
    #[error("可信Unix入口配置或监听失败；不覆盖既有路径")]
    Ingress,
    #[error("既有canonical写者或完整监视视图不可用；不初始化、不修复、不回退")]
    Writer,
}

impl StartupError {
    /// 路由装配失败按错误种类映射成固定类别；逐项匹配，新增种类时编译器会要求补上。
    pub(crate) fn router(error: &LocalError) -> Self {
        let reason = match error {
            LocalError::Read(_) => "只读副本读取失败",
            LocalError::Incomplete { .. } => "副本扫描不完整",
            LocalError::Encoding(_) => "副本含非UTF-8文档",
            LocalError::Index(_) => "索引构建失败",
            LocalError::Tree(_) => "文件树构建失败",
            LocalError::Configuration => "本机端口或副本名称无效",
            LocalError::Unavailable => "完整视图暂不可用",
        };
        Self::Router { reason }
    }
}
