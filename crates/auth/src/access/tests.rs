//! Access Interface 的真实签名、完整声明与固定配置契约。

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::json;

use super::{
    AccessConfig, AccessError,
    test_support::{PRIVATE_KEY, claims, config, signed, token, verifier},
};

#[tokio::test]
async fn signed_claims_supply_identity_only_after_verification() {
    let verifier = verifier().await;
    let payload = claims();
    let identity = verifier.verify(&signed(&payload)).await.unwrap();
    assert_eq!(identity.email(), "maxsong@example.com");
    assert_eq!(identity.subject(), "fictional-person");
    assert_eq!(identity.issuer(), "https://studiolo.cloudflareaccess.com");
    assert_eq!(identity.audience(), "example-audience");
    assert_eq!(identity.expires_at(), payload["exp"].as_u64().unwrap());
}

#[tokio::test]
async fn configured_audience_is_stable_when_signed_audiences_are_reordered() {
    let verifier = verifier().await;
    for audiences in [
        json!(["other-application", "example-audience"]),
        json!(["example-audience", "other-application"]),
    ] {
        let mut payload = claims();
        payload["aud"] = audiences;
        let identity = verifier.verify(&signed(&payload)).await.unwrap();
        assert_eq!(identity.audience(), "example-audience");
    }
}

#[tokio::test]
async fn public_fixture_verifies_signatures_but_cannot_select_a_real_owner() {
    let verifier = super::fixture::fixture_verifier().await.unwrap();
    assert_eq!(verifier.origin(), "https://studiolo.example.com");
    assert!(verifier.verify(&signed(&claims())).await.is_ok());
    for (field, value) in [
        ("iss", json!("https://actual-owner.cloudflareaccess.com")),
        ("aud", json!(["actual-application"])),
        ("email", json!("owner@actual.example")),
    ] {
        let mut payload = claims();
        payload[field] = value;
        assert_eq!(
            verifier.verify(&signed(&payload)).await,
            Err(AccessError::Rejected)
        );
    }
}

#[tokio::test]
async fn wrong_claims_missing_claims_and_wrong_types_are_rejected() {
    let verifier = verifier().await;
    for (field, value) in [
        ("iss", json!("https://evil.cloudflareaccess.com")),
        ("aud", json!(["different-application"])),
        ("aud", json!([])),
        ("aud", json!("example-audience")),
        ("email", json!("attacker@example.com")),
        ("email", json!("MAXSONG@example.com")),
        ("type", json!("org")),
        ("type", json!("service")),
        ("sub", json!("")),
        ("sub", json!("  ")),
        ("sub", json!("person\n")),
        ("sub", json!("a".repeat(257))),
        ("exp", json!(1)),
        ("exp", json!(-1)),
        ("exp", json!("9999999999")),
        ("nbf", json!(9999999999_u64)),
    ] {
        let mut bad = claims();
        bad[field] = value;
        assert_eq!(
            verifier.verify(&signed(&bad)).await,
            Err(AccessError::Rejected),
            "{field}"
        );
    }
    for field in ["iss", "aud", "email", "type", "sub", "exp", "nbf"] {
        let mut bad = claims();
        bad.as_object_mut().unwrap().remove(field);
        assert_eq!(
            verifier.verify(&signed(&bad)).await,
            Err(AccessError::Rejected),
            "missing {field}"
        );
    }
}

#[tokio::test]
async fn expiry_at_current_second_has_no_leeway() {
    let verifier = verifier().await;
    let mut expired = claims();
    expired["exp"] = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs())
    .into();
    assert_eq!(
        verifier.verify(&signed(&expired)).await,
        Err(AccessError::Rejected)
    );
}

#[tokio::test]
async fn algorithms_signatures_and_untrusted_key_selectors_fail_closed() {
    let verifier = verifier().await;
    let claims = claims();
    let mut header = Header::new(Algorithm::RS256);
    // 真实签名缺少 kid 仍须失败；不采用默认或回退密钥。
    assert_eq!(
        verifier.verify(&token(&header, &claims)).await,
        Err(AccessError::Rejected)
    );
    for kid in ["", "unknown", "../test-key", &"a".repeat(129)] {
        header.kid = Some(kid.into());
        assert_eq!(
            verifier.verify(&token(&header, &claims)).await,
            Err(AccessError::Rejected)
        );
    }
    header.kid = Some("test-key".into());
    header.alg = Algorithm::RS384;
    assert_eq!(
        verifier.verify(&token(&header, &claims)).await,
        Err(AccessError::Rejected)
    );
    header.alg = Algorithm::HS256;
    let confused = encode(&header, &claims, &EncodingKey::from_secret(PRIVATE_KEY)).unwrap();
    assert_eq!(verifier.verify(&confused).await, Err(AccessError::Rejected));
    header.alg = Algorithm::RS256;
    header.jku = Some("https://attacker.example/keys".into());
    assert_eq!(
        verifier.verify(&token(&header, &claims)).await,
        Err(AccessError::Rejected)
    );
    header.jku = None;
    header.crit = Some(vec!["unsupported".into()]);
    assert_eq!(
        verifier.verify(&token(&header, &claims)).await,
        Err(AccessError::Rejected)
    );
    let valid = signed(&claims);
    let (message, signature) = valid.rsplit_once('.').unwrap();
    let corrupted = format!(
        "{message}.{}{}",
        if signature.starts_with('A') { "B" } else { "A" },
        &signature[1..]
    );
    assert_eq!(
        verifier.verify(&corrupted).await,
        Err(AccessError::Rejected)
    );
    for malformed in [
        "",
        "a.b.c",
        "eyJhbGciOiJub25lIn0.e30.",
        &"a".repeat(16 * 1024 + 1),
    ] {
        assert_eq!(verifier.verify(malformed).await, Err(AccessError::Rejected));
    }
}

#[test]
fn configuration_is_exact_and_cannot_select_an_arbitrary_jwks_host() {
    let good = config();
    for public in [
        "http://studiolo.example.com",
        "https://studiolo.example.com/",
        "https://studiolo.example.com/path",
        "https://user@studiolo.example.com",
        "https://studiolo.example.com?x=1",
        "https://studiolo.example.com#x",
        "https://127.0.0.1",
        "https://localhost",
        "https://STUDIOLO.example.com",
    ] {
        assert!(
            AccessConfig::new(
                public.into(),
                good.issuer.clone(),
                good.audience.clone(),
                good.email.clone()
            )
            .is_err()
        );
    }
    for issuer in [
        "https://cloudflareaccess.com",
        "http://studiolo.cloudflareaccess.com",
        "https://a.b.cloudflareaccess.com",
        "https://-a.cloudflareaccess.com",
        "https://a-.cloudflareaccess.com",
        "https://a.cloudflareaccess.com/",
        "https://a.cloudflareaccess.com:443",
        "https://a.cloudflareaccess.com.evil.example",
        "https://user@a.cloudflareaccess.com",
        "https://127.0.0.1",
    ] {
        assert!(
            AccessConfig::new(
                good.public_origin.clone(),
                issuer.into(),
                good.audience.clone(),
                good.email.clone()
            )
            .is_err()
        );
    }
    for email in ["", "person", "a@@b.com", "a @b.com", "a@b.com\n"] {
        assert!(
            AccessConfig::new(
                good.public_origin.clone(),
                good.issuer.clone(),
                good.audience.clone(),
                email.into()
            )
            .is_err()
        );
    }
    for audience in ["", "has space", "bad\n", &"a".repeat(257)] {
        assert!(
            AccessConfig::new(
                good.public_origin.clone(),
                good.issuer.clone(),
                audience.into(),
                good.email.clone()
            )
            .is_err()
        );
    }
}
