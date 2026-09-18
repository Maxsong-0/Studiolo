//! 健康页复用既有会话门禁：不新增无鉴权入口，也不在缺证据时伪造结论。
//!
//! 这里用的就是普通会话入口的同一套夹具（`router_with_sessions`），
//! 所以证明的是"健康页和其它内容 API 走同一条门禁"，不是另起一套断言。

use super::support::*;

/// 健康页固定路径；与 `crate::health::PATH` 同一串。
const HEALTH: &str = "/api/v1/health";

#[tokio::test]
async fn 匿名请求健康页被既有会话门禁拒绝而不是拿到任何结论() {
    let f = Fixture::new(true).await;

    // 不带应用 Cookie，其余头一律齐备：被拒的只可能是会话门禁本身。
    let response = f.send(f.request("GET", HEALTH, None, ""), 401).await;

    let body = json(response).await.to_string();
    assert!(!body.contains("overall"), "被拒的请求不得带出任何健康结论");
    assert!(!body.contains("sections"));
}

#[tokio::test]
async fn 缺少可信access身份时健康页同样拿不到结论() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    let mut request = f.request("GET", HEALTH, Some(&issued.cookie), "");
    request.headers_mut().remove("cf-access-jwt-assertion");

    let response = f.send(request, 401).await;

    let body = json(response).await.to_string();
    assert!(!body.contains("overall"));
}

#[tokio::test]
async fn 跨站来源的健康页请求按既有同源策略被拒() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    let mut request = f.request("GET", HEALTH, Some(&issued.cookie), "");
    request
        .headers_mut()
        .insert("sec-fetch-site", "cross-site".parse().unwrap());

    let response = f.send(request, 403).await;

    let body = json(response).await.to_string();
    assert!(!body.contains("overall"));
}

#[tokio::test]
async fn 持有效会话但未装配证据来源时健康页回404因而不存在无证据的假结论() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;

    // 本夹具直接调 `router_with_sessions`，没有可信启动层那一步
    // `with_health_evidence`，所以证据来源确实缺席。
    let response = f
        .send(f.request("GET", HEALTH, Some(&issued.cookie), ""), 404)
        .await;

    let body = json(response).await;
    assert_eq!(body["error"], "本机未装配健康证据来源。");
    assert!(
        body.get("overall").is_none(),
        "没有证据来源时绝不能给出整体状态"
    );
}

#[tokio::test]
async fn 健康页只接受只读方法且405响应的allow头只列出get() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;

    let response = f
        .send(f.request("POST", HEALTH, Some(&issued.cookie), "{}"), 405)
        .await;

    assert_eq!(response.headers()["allow"], "GET");
}
