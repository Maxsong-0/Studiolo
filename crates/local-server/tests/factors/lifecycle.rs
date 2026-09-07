//! 真实HTTP完成提权、恢复码交付和因子切换，原请求未读取响应也不重放恢复。

use super::{client::Client, fixture::Fixture, support::*};
use std::time::Duration;
use studiolo_auth::Totp;

#[tokio::test]
async fn unix_http_step_up_codes_and_recovery_revoke_every_old_session() {
    let mut f = Fixture::new(true).await;
    let first = f.issue().await;
    let second = f.issue().await;
    f.elevate(&first, false).await;
    f.send(f.request("POST", CODES, Some(&second), "{}"), 403)
        .await
        .kept_cookie();
    let response = f
        .send(f.request("POST", CODES, Some(&first), "{}"), 200)
        .await;
    response.kept_cookie();
    let body = response.json();
    assert_eq!(body.as_object().unwrap().len(), 1);
    let codes = body["codes"].as_array().unwrap();
    assert_eq!(codes.len(), 10);
    for code in codes {
        assert_eq!(code.as_str().unwrap().len(), 55);
    }
    let audit = format!("{:?}", f.events("recovery.codes_rotated"));
    for code in codes {
        assert!(!audit.contains(code.as_str().unwrap()));
    }
    f.advance(60_000);
    let body = serde_json::json!({"recovery_code":codes[0], "code":otp(&NEW, f.now())}).to_string();
    let recovered = f
        .send(f.request("POST", RECOVER, Some(&first), &body), 204)
        .await;
    recovered.cleared_cookie();
    assert!(recovered.body.is_empty());
    for old in [&first, &second] {
        f.send(f.request("GET", OWN, Some(old), ""), 401)
            .await
            .cleared_cookie();
    }
    let fresh = f.issue().await;
    f.advance(60_000);
    f.elevate(&fresh, true).await;
    let codes = f
        .send(f.request("POST", CODES, Some(&fresh), "{}"), 200)
        .await
        .json();
    let body = serde_json::json!({"recovery_code":codes["codes"][0], "code":otp(&OLD, f.now())})
        .to_string();
    f.send(f.request("POST", RECOVER, Some(&fresh), &body), 409)
        .await
        .kept_cookie();
    assert_eq!(f.events("totp.recovered").len(), 2);
    f.stop().await;
}

#[tokio::test]
async fn unread_recovery_response_reopens_with_replacement_and_never_reissues_it() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    f.elevate(&login, false).await;
    let codes = f
        .send(f.request("POST", CODES, Some(&login), "{}"), 200)
        .await
        .json();
    f.advance(60_000);
    let body = serde_json::json!({"recovery_code":codes["codes"][0], "code":otp(&NEW, f.now())})
        .to_string();
    let mut connection = Client::connect(&f.socket).await;
    connection
        .write(f.request("POST", RECOVER, Some(&login), &body))
        .await;
    // 只查询公开当前因子绑定确认提交，不读取HTTP响应或重放恢复请求。
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if Totp::attach(f.manager(), seed(true)).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(connection);
    f.restart().await;
    f.send(f.request("GET", OWN, Some(&login), ""), 401)
        .await
        .cleared_cookie();
    let fresh = f.issue().await;
    f.advance(60_000);
    f.elevate(&fresh, true).await;
    f.send(f.request("POST", RECOVER, Some(&fresh), &body), 409)
        .await
        .kept_cookie();
    assert_eq!(f.events("totp.recovered").len(), 2);
    f.stop().await;
}

#[tokio::test]
async fn missing_candidate_and_old_code_batch_do_not_clear_valid_cookies() {
    let mut f = Fixture::new(false).await;
    let login = f.issue().await;
    f.elevate(&login, false).await;
    let codes = f
        .send(f.request("POST", CODES, Some(&login), "{}"), 200)
        .await
        .json();
    let body = serde_json::json!({"recovery_code":codes["codes"][0], "code":otp(&NEW, f.now())})
        .to_string();
    f.send(f.request("POST", RECOVER, Some(&login), &body), 409)
        .await
        .kept_cookie();
    f.send(f.request("GET", OWN, Some(&login), ""), 200).await;
    f.stop().await;

    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    f.elevate(&login, false).await;
    let old = f
        .send(f.request("POST", CODES, Some(&login), "{}"), 200)
        .await
        .json();
    let new = f
        .send(f.request("POST", CODES, Some(&login), "{}"), 200)
        .await
        .json();
    f.advance(60_000);
    let body = |codes: &serde_json::Value| {
        serde_json::json!({
            "recovery_code":codes["codes"][0], "code":otp(&NEW, f.now()),
        })
        .to_string()
    };
    f.send(f.request("POST", RECOVER, Some(&login), &body(&old)), 403)
        .await
        .kept_cookie();
    f.send(f.request("POST", RECOVER, Some(&login), &body(&new)), 204)
        .await
        .cleared_cookie();
    f.stop().await;
}
