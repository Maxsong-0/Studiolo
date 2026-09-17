use serde::{Deserialize, Deserializer};
use studiolo_core::VaultPath;

#[derive(Clone)]
pub(in crate::file_operations::rename) struct Rule {
    pub(in crate::file_operations::rename) path: VaultPath,
    pub(in crate::file_operations::rename) class: String,
    pub(in crate::file_operations::rename) max_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Object {
    path: VaultPath,
    // String 精确解码；拒绝 serde unit enum 可接受的外部标记对象。
    class: String,
    max_bytes: u64,
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let rule: Object = super::object(input)?;
        Ok(Self {
            path: rule.path,
            class: rule.class,
            max_bytes: rule.max_bytes,
        })
    }
}
