//! 生产仓库能力只来自私有主人配置，不把文档声明或环境值当作宿主路径授权。
use super::{Config, ConfigError, Path, PathBuf, Root, ServingRoots, config_file};
use serde::{Deserialize, Deserializer};
use studiolo_core::VaultPath;

#[derive(Clone)]
pub(crate) struct RepositoryScope(Document);

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    project: VaultPath,
    directory: PathBuf,
    #[serde(default)]
    cargo_dependencies: bool,
}

pub(crate) fn decode_repository<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<Option<RepositoryScope>, D::Error> {
    struct Object;
    impl<'de> serde::de::Visitor<'de> for Object {
        type Value = Option<RepositoryScope>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("显式只读仓库批准对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
            Document::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                .map(|document| Some(RepositoryScope(document)))
        }
    }
    input.deserialize_map(Object)
}

impl RepositoryScope {
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        let parts: Vec<_> = self.0.project.as_str().split('/').collect();
        if self.0.version != 1
            || !crate::absolute_normal(&self.0.directory)
            || parts.len() != 2
            || parts[0] != "项目"
            || parts[1].starts_with('.')
            || self.0.project.is_protected()
        {
            return Err(ConfigError);
        }
        Ok(())
    }
}

/// 配置、仓库及受保护服务根的封闭批准；无公开构造器或反序列化入口。
///
/// 源码解析没有在此类型中获得生产授权；seed_ref 始终只是逻辑标识。
/// 不从环境值推断来源文件，也不声称已验证任意环境秘密来源。
/// ```compile_fail
/// let roots: studiolo_host_config::RepositoryServingRoots = serde_json::from_str("{}").unwrap();
/// ```
pub struct RepositoryServingRoots {
    base: ServingRoots,
    repository: Root,
    configuration: Root,
    writer: Option<Root>,
    ingress: Option<Root>,
    scope: RepositoryScope,
}

impl RepositoryServingRoots {
    pub(crate) fn new(
        config: &Config,
        scope: &RepositoryScope,
        vault: &Path,
        assets: &Path,
        writer_data: Option<&Path>,
        ingress_socket: Option<&Path>,
    ) -> Result<Self, ConfigError> {
        scope.validate()?;
        let base = ServingRoots::new(config, vault, assets)?;
        let ingress = ingress_socket
            .map(|socket| {
                if !crate::absolute_normal(socket) {
                    return Err(ConfigError);
                }
                Root::new(socket.parent().ok_or(ConfigError)?, true)
            })
            .transpose()?;
        let approved = Self {
            base,
            repository: Root::new(&scope.0.directory, false)?,
            configuration: Root::new(config.file.path.parent().ok_or(ConfigError)?, true)?,
            writer: writer_data.map(|path| Root::new(path, true)).transpose()?,
            ingress,
            scope: scope.clone(),
        };
        approved.revalidate()?;
        Ok(approved)
    }

    /// 只供可信装配层固定项目标识，不接受查询参数覆盖。
    #[must_use]
    pub fn project(&self) -> &str {
        self.scope.0.project.as_str()
    }

    /// 批准的唯一根，调用者仍须比较实际打开的 Broker 句柄身份。
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.repository.path
    }

    /// 固定设备/inode，不代表任意路径的读取授权。
    #[must_use]
    pub fn root_identity(&self) -> (u64, u64) {
        self.repository.identity
    }

    /// Cargo 必须单独声明 true；源码权限永不由本对象授予。
    #[must_use]
    pub fn cargo_dependencies(&self) -> bool {
        self.scope.0.cargo_dependencies
    }

    fn private_roots(&self) -> impl Iterator<Item = &Root> {
        std::iter::once(&self.configuration)
            .chain(self.writer.iter())
            .chain(self.ingress.iter())
    }

    /// 启动、秘密装配及查询发布前复核；目录内容更新不改变根身份。
    pub fn revalidate(&self) -> Result<(), ConfigError> {
        self.base.revalidate()?;
        self.repository.check()?;
        for root in self.private_roots() {
            root.check()?;
        }
        for root in self.base.roots.iter().chain(self.private_roots()) {
            config_file::disjoint(&self.repository.path, &root.path)?;
        }
        if let Some(writer) = &self.writer {
            for root in &self.base.roots {
                config_file::disjoint(&writer.path, &root.path)?;
            }
            config_file::outside(&self.base.file.path, writer.identity)?;
        }
        if let Some(ingress) = &self.ingress {
            for root in self
                .base
                .roots
                .iter()
                .chain(std::iter::once(&self.configuration))
                .chain(self.writer.iter())
            {
                config_file::disjoint(&ingress.path, &root.path)?;
            }
        }
        // 分离关系检查也读取路径，返回前不能悄悄接纳中途替换后的新基准。
        self.repository.check()?;
        for root in self.private_roots() {
            root.check()?;
        }
        self.base.revalidate()
    }
}
