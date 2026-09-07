# 因子显示状态（T-025）

本契约定义非授权状态读取；实测结果以[M2记录](../m2/README.md)为准。
不扩展[bootstrap及会话响应](frontend-session.md)的原有形状。

所有显式session组合均提供`GET /api/v1/auth/state`，要求真实Access、有效配对L2及
严格同源Fetch/自定义头；只接受GET，无query、无正文，沿原头/帧/时间预算。
GET不接受新的凭据输入，也不需要因子动作的来源IP。运行故障不能返回未启用能力。

```json
{"factors":null}
```

仅表示可信组合根没有装配因子HTTP。启用时为以下封闭对象：

```json
{
  "factors": {
    "observed_at_ms": 0,
    "recent_authentication": null,
    "recovery_candidate_available": false,
    "pending": false
  }
}
```

`recent_authentication`存在时只有`verified_at_ms`、`expires_at_ms`。时间单位为Unix毫秒，
有效期不包含边界点。没有种子、引用、摘要、码余量、批次/因子序号、审计回执或通用授权票据。

Auth的`FactorRuntime::state(identity,token,csrf)`在一次原会话事务内验证当前凭据、
选择权威活动槽和读取投影；复用动作的活动槽选择及现有近期认证规则。观察到替换槽后
仍只收紧旧槽资格。未知绑定、坏库和错误凭据失败关闭；HTTP不检查表或解释内部状态。

GET保留原时钟地板、到期观察与会话活跃时间语义，不扣因子验证预算、消费OTP、换码、
清理pending、补记审计或激活近期认证。`pending`包括尚未清理的过期决定；它不是
某个HTTP请求是否结束的证明，false也不覆盖尚未进入持久决定阶段的在途操作。

读取中发现会话撤销返回401并清Cookie。其余运行故障按会话错误规则返回，保留可能有效
Cookie，不返回`factors:null`。安全响应头及no-store不变；旧无session组合没有此能力。

客户端可以显示当前快照；服务端仍在每个动作中重新授权。响应丢失后，值未变化不证明
原动作失败，值变化也不能恢复原码或替代明确交付。任何自动重放POST均不属于本契约。
真实身份、HTTPS、手机配对与部署仍需独立验收。
