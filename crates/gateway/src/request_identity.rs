//! 固定宽度的请求身份；不保留参数或 prompt，JSON 身份遵守 Value 的相等语义。

use crate::{ModelRequest, ToolCall};
use serde_json::{Number, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RequestIdentity([u8; 32]);

impl RequestIdentity {
    pub(crate) fn tool(call: &ToolCall) -> Self {
        let mut digest = IdentityHasher::new(b"gateway.tool-request.v1");
        digest.bytes(call.call_id().as_str().as_bytes());
        digest.bytes(call.tool().as_str().as_bytes());
        digest.value(call.arguments());
        Self(digest.0.finalize().into())
    }

    pub(crate) fn model(request: &ModelRequest) -> Self {
        let mut digest = IdentityHasher::new(b"gateway.model-request.v1");
        for field in [
            request.call_id().as_str(),
            request.provider(),
            request.model(),
            request.prompt(),
        ] {
            digest.bytes(field.as_bytes());
        }
        Self(digest.0.finalize().into())
    }
}

struct IdentityHasher(Sha256);

impl IdentityHasher {
    fn new(domain: &[u8]) -> Self {
        let mut digest = Self(Sha256::new());
        digest.bytes(domain);
        digest
    }

    fn length(&mut self, length: usize) {
        self.0.update((length as u64).to_be_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.length(bytes.len());
        self.0.update(bytes);
    }

    fn value(&mut self, value: &Value) {
        match value {
            Value::Null => self.0.update(b"n"),
            Value::Bool(value) => self.0.update(if *value { b"t" } else { b"f" }),
            Value::Number(number) => {
                self.0.update(b"d");
                // Number 区分整数与浮点数；有限浮点数中只有 +/-0 的相等值有
                // 不同的字符串。直接按 Number::eq 判零，也保留 arbitrary_precision
                // 启用时的字符串相等语义（该模式的 -0.0 与 0.0 不相等）。
                let zero = Number::from_f64(0.0).expect("零值始终是有限浮点数");
                let canonical = if number == &zero {
                    "0.0".to_owned()
                } else {
                    number.to_string()
                };
                self.bytes(canonical.as_bytes());
            }
            Value::String(value) => {
                self.0.update(b"s");
                self.bytes(value.as_bytes());
            }
            Value::Array(values) => {
                self.0.update(b"a");
                self.length(values.len());
                for value in values {
                    self.value(value);
                }
            }
            Value::Object(values) => {
                self.0.update(b"o");
                self.length(values.len());
                // 即使工作区 feature 合并启用 preserve_order，对象键序也不影响身份。
                let mut entries: Vec<_> = values.iter().collect();
                entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
                for (key, value) in entries {
                    self.bytes(key.as_bytes());
                    self.value(value);
                }
            }
        }
    }
}
