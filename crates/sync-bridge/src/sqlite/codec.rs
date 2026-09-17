//! 明确长度前缀的 v1 二进制元数据；先预算再分配，未知标签与尾随字节拒绝。

use super::{BridgeError, STATE_MAX, model::Result};

pub(crate) struct Writer {
    pub bytes: Vec<u8>,
    cap: usize,
}
impl Writer {
    pub fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
        }
    }
    pub fn raw(&mut self, value: &[u8]) -> Result<()> {
        if self
            .bytes
            .len()
            .checked_add(value.len())
            .is_none_or(|n| n > self.cap)
        {
            return Err(BridgeError::Budget);
        }
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    pub fn byte(&mut self, value: u8) -> Result<()> {
        self.raw(&[value])
    }
    pub fn num(&mut self, value: u64) -> Result<()> {
        i64::try_from(value).map_err(|_| BridgeError::Overflow)?;
        self.raw(&value.to_be_bytes())
    }
    pub fn blob(&mut self, value: &[u8], max: usize) -> Result<()> {
        if value.len() > max {
            return Err(BridgeError::Budget);
        }
        self.num(value.len() as u64)?;
        self.raw(value)
    }
    pub fn text(&mut self, value: &str, max: usize) -> Result<()> {
        self.blob(value.as_bytes(), max)
    }
    pub fn option<T>(
        &mut self,
        value: &Option<T>,
        f: impl FnOnce(&mut Self, &T) -> Result<()>,
    ) -> Result<()> {
        self.byte(u8::from(value.is_some()))?;
        if let Some(value) = value {
            f(self, value)?;
        }
        Ok(())
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() > STATE_MAX {
            return Err(BridgeError::Budget);
        }
        Ok(Self { bytes, offset: 0 })
    }
    pub fn raw(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or(BridgeError::Budget)?;
        let result = self
            .bytes
            .get(self.offset..end)
            .ok_or(BridgeError::CorruptStore)?;
        self.offset = end;
        Ok(result)
    }
    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.raw(1)?[0])
    }
    pub fn num(&mut self) -> Result<u64> {
        let value = u64::from_be_bytes(
            self.raw(8)?
                .try_into()
                .map_err(|_| BridgeError::CorruptStore)?,
        );
        i64::try_from(value).map_err(|_| BridgeError::Overflow)?;
        Ok(value)
    }
    pub fn count(&mut self, max: usize) -> Result<usize> {
        let n = usize::try_from(self.num()?).map_err(|_| BridgeError::Budget)?;
        if n > max {
            return Err(BridgeError::Budget);
        }
        Ok(n)
    }
    pub fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.raw(N)?
            .try_into()
            .map_err(|_| BridgeError::CorruptStore)
    }
    pub fn blob(&mut self, max: usize) -> Result<&'a [u8]> {
        let n = self.count(max)?;
        self.raw(n)
    }
    pub fn text(&mut self, max: usize) -> Result<String> {
        String::from_utf8(self.blob(max)?.to_vec()).map_err(|_| BridgeError::CorruptStore)
    }
    pub fn option<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        match self.byte()? {
            0 => Ok(None),
            1 => Ok(Some(f(self)?)),
            _ => Err(BridgeError::CorruptStore),
        }
    }
    pub fn end(self) -> Result<()> {
        if self.offset != self.bytes.len() {
            return Err(BridgeError::CorruptStore);
        }
        Ok(())
    }
}

pub(crate) fn json<T: serde::Serialize>(value: &T, max: usize) -> Result<String> {
    // serde 的写入器逐块拒绝上限，不先生成无限大的 String。
    struct Limited(Writer);
    impl std::io::Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .raw(bytes)
                .map_err(|_| std::io::Error::other("metadata budget"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Limited(Writer::new(max));
    serde_json::to_writer(&mut out, value).map_err(|_| BridgeError::Budget)?;
    String::from_utf8(out.0.bytes).map_err(|_| BridgeError::CorruptStore)
}
