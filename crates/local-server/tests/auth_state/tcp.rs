use super::{client::Reply, common::*, fixture::Fixture, test_support::verifier};
use std::{sync::Arc, time::Duration};
use studiolo_local_server::{router_with_canonical_sessions, router_with_sessions};
use studiolo_vault_broker::{CanonicalReadVault, FsVaultBroker, ReadOnlyVault};

#[tokio::test]
async fn old_tcp_session_compositions_return_only_explicitly_unconfigured_factors() {
    let mut fixture = Fixture::new(true).await;
    let login = fixture.issue().await;
    fixture.stop().await;
    for canonical in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let vault = fixture.root.join("vault");
        let assets = ReadOnlyVault::open(&fixture.root.join("web")).unwrap();
        let app = if canonical {
            // 仅初始化虚构夹具的身份；旧组合本身仍为只读、无因子秘密能力。
            drop(FsVaultBroker::open(&vault, &fixture.root.join("broker-data")).unwrap());
            router_with_canonical_sessions(
                (
                    ReadOnlyVault::open(&vault).unwrap(),
                    Arc::new(CanonicalReadVault::open_existing(&vault).unwrap()),
                ),
                assets,
                "虚构状态夹具".into(),
                address.port(),
                verifier().await,
                fixture.manager(),
            )
            .unwrap()
        } else {
            router_with_sessions(
                ReadOnlyVault::open(&vault).unwrap(),
                assets,
                "虚构状态夹具".into(),
                address.port(),
                verifier().await,
                fixture.manager(),
            )
            .unwrap()
        };
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let mut input = request(&fixture, Some(&login));
        input
            .headers_mut()
            .insert("host", address.to_string().parse().unwrap());
        let response = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(6))
            .build()
            .unwrap()
            .get(format!("http://{address}{STATE}"))
            .headers(input.headers().clone())
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await.unwrap().to_vec();
        let reply = Reply {
            status,
            headers,
            body,
        };
        assert_eq!(reply.status, 200);
        reply.secure();
        reply.kept_cookie();
        no_secrets(&reply, &login, &[]);
        let value = reply.json();
        keys(&value, &["factors"]);
        assert!(value["factors"].is_null());
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(6), server)
            .await
            .unwrap()
            .unwrap();
    }
}
