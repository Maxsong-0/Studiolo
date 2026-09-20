//! 私有 Unix 转发：固定地址、无代理、无重定向；外壳不获得 canonical 文件能力。
use std::{path::Path, time::Duration};

/// 通向既有 canonical 单写者的有界连接。
pub struct BoardClient {
    client: reqwest::Client,
}

impl BoardClient {
    /// 从明确配置的绝对 Unix socket 构造；权限和 peer 由接收端核验。
    pub fn new(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() || path.as_os_str().len() > 103 {
            return Err("黑板 Unix 路径无效".into());
        }
        let client = reqwest::Client::builder()
            .unix_socket(path.to_owned())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "黑板连接配置失败".to_owned())?;
        Ok(Self { client })
    }

    /// 仅向固定入口转发结构化请求，限制请求和响应尺寸。
    pub async fn call(&self, request: &serde_json::Value) -> Result<serde_json::Value, String> {
        let bytes = serde_json::to_vec(request).map_err(|_| "黑板请求不可编码")?;
        if bytes.len() > 262_144 {
            return Err("黑板请求超过上限".into());
        }
        let mut response = self
            .client
            .post("http://localhost/internal/board")
            .header("content-type", "application/json")
            .header("cf-connecting-ip", "127.0.0.1")
            .body(bytes)
            .send()
            .await
            .map_err(|_| "黑板权威服务不可用；停止协作")?;
        let success = response.status().is_success();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "黑板响应不可用")? {
            if body.len() + chunk.len() > 1_048_576 {
                return Err("黑板响应超过上限".into());
            }
            body.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| "黑板响应无效".to_owned())?;
        if !success {
            return Err(value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("黑板请求被权威服务拒绝")
                .chars()
                .take(512)
                .collect());
        }
        Ok(value)
    }
}
