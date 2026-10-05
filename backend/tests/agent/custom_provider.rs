use super::super::{ProviderCatalog, ProviderToolCall};
use super::*;
use axum::{routing::post, Router};
use std::sync::atomic::{AtomicUsize, Ordering};

const TEST_KEY: &str = "fixture-only-custom-secret";
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    store: CustomProviderStore,
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "jx3-custom-provider-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let store = CustomProviderStore::default();
        store.configure(dir.join("provider.json"), true);
        Self { store, dir }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.dir.join("provider.json"));
        let _ = std::fs::remove_file(self.dir.join("provider.json.tmp"));
        let _ = std::fs::remove_dir(&self.dir);
    }
}
fn input(url: &str, key: &str, protocol: CustomProtocol) -> CustomInput {
    CustomInput {
        label: "我的接口".into(),
        model: "fixture-model".into(),
        base_url: url.into(),
        api_key: key.into(),
        protocol,
    }
}

#[test]
fn custom_metadata_survives_restart_but_key_does_not() {
    let fixture = Fixture::new();
    fixture
        .store
        .save(input(
            "https://api.example.com/v1/chat/completions/",
            TEST_KEY,
            CustomProtocol::ChatCompletions,
        ))
        .unwrap();
    let disk = std::fs::read_to_string(fixture.dir.join("provider.json")).unwrap();
    assert!(!disk.contains(TEST_KEY));
    assert!(!disk.contains("api_key"));
    assert!(!fixture.store.view().to_string().contains(TEST_KEY));
    assert!(!format!("{:?}", fixture.store).contains(TEST_KEY));
    assert_eq!(
        fixture.store.view()["config"]["base_url"],
        "https://api.example.com/v1"
    );
    assert!(fixture.store.safe_profile().unwrap().available);
    let restored = CustomProviderStore::default();
    restored.configure(fixture.dir.join("provider.json"), true);
    assert_eq!(restored.view()["config"], fixture.store.view()["config"]);
    assert!(!restored.safe_profile().unwrap().available);
    assert_eq!(
        restored.create_provider().err().unwrap().code,
        "provider_key_unavailable"
    );
    fixture.store.remove().unwrap();
    assert!(fixture.store.safe_profile().is_none());
    assert!(!fixture.dir.join("provider.json").exists());
}

#[test]
fn custom_edit_reuses_key_only_for_same_endpoint_and_is_transactional() {
    let f = Fixture::new();
    f.store
        .save(input(
            "https://api.example.com/v1",
            TEST_KEY,
            CustomProtocol::ChatCompletions,
        ))
        .unwrap();
    let mut edit = input("https://api.example.com/v1/", "", CustomProtocol::Responses);
    edit.model = "second-model".into();
    f.store.save(edit).unwrap();
    assert!(f.store.safe_profile().unwrap().available);
    let before = std::fs::read(f.dir.join("provider.json")).unwrap();
    assert_eq!(
        f.store
            .save(input(
                "https://different.example.com/v1",
                "",
                CustomProtocol::Responses
            ))
            .unwrap_err()
            .code,
        "custom_key_required"
    );
    assert_eq!(
        f.store
            .save(input(
                "https://api.example.com/v2",
                "",
                CustomProtocol::Responses
            ))
            .unwrap_err()
            .code,
        "custom_key_required"
    );
    assert_eq!(std::fs::read(f.dir.join("provider.json")).unwrap(), before);
    assert_eq!(
        f.store
            .save(input(
                "https://api.example.com/v1",
                "a\nb",
                CustomProtocol::Responses
            ))
            .unwrap_err()
            .code,
        "custom_key_invalid"
    );
}

#[test]
fn custom_profiles_are_worker_scoped_and_catalog_selectable() {
    let a = ProviderCatalog::offline_default();
    let b = ProviderCatalog::offline_default();
    a.custom
        .save(input(
            "https://api.example.com/v1",
            TEST_KEY,
            CustomProtocol::Deepseek,
        ))
        .unwrap();
    assert_eq!(a.safe_list().profiles.len(), 2);
    assert_eq!(b.safe_list().profiles.len(), 1);
    let snapshot = a.create_provider(CUSTOM_PROFILE_ID).unwrap();
    assert_eq!(snapshot.model(), "fixture-model");
    a.custom.remove().unwrap();
    assert_eq!(snapshot.model(), "fixture-model");
    assert!(a.create_provider(CUSTOM_PROFILE_ID).is_err());
    assert!(a.create_provider("offline").is_ok());
    let json = serde_json::to_string(&a.safe_list()).unwrap();
    assert!(!json.contains(TEST_KEY));
    assert!(!json.contains("api.example.com"));
}

#[test]
fn custom_url_rejects_private_addresses_credentials_and_insecure_remote() {
    for url in [
        "http://api.example.com/v1",
        "https://u:p@api.example.com/v1",
        "https://api.example.com/v1?key=abc",
        "https://api.example.com/#x",
        "https://127.0.0.1/v1",
        "https://[::1]/v1",
        "https://10.0.0.1/v1",
        "https://169.254.169.254/v1",
        "https://[::ffff:127.0.0.1]/v1",
        "https://[fc00::1]/v1",
        "file:///key",
        "https://2130706433/v1",
    ] {
        assert!(custom_network::normalize_url(url, false).is_err(), "{url}");
    }
    assert!(custom_network::normalize_url("http://127.0.0.1:8000/v1", true).is_ok());
    assert!(custom_network::normalize_url("http://[::1]:8000/v1", true).is_ok());
    assert!(custom_network::normalize_url("http://localhost:8000/v1", true).is_ok());
    assert!(custom_network::normalize_url("http://192.168.1.1/v1", true).is_err());
    for ip in [
        "0.0.0.0",
        "100.64.0.1",
        "198.18.0.1",
        "192.0.2.1",
        "203.0.113.1",
        "224.0.0.1",
        "2001:db8::1",
        "2002:7f00:1::",
        "3fff::1",
    ] {
        assert!(!custom_network::public_ip(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
        assert!(custom_network::public_ip(ip.parse().unwrap()));
    }
}

#[test]
fn custom_inputs_and_browser_guard_do_not_echo_credentials() {
    let malformed = format!(r#"{{"api_key":"{TEST_KEY}","extra":1}}"#);
    assert!(
        !serde_json::to_string(&parse_input(malformed.as_bytes()).err().unwrap())
            .unwrap()
            .contains(TEST_KEY)
    );
    assert!(parse_input(&vec![b'a'; MAX_BODY_BYTES + 1]).is_err());
    let mut h = HeaderMap::new();
    assert!(check_headers(&h).is_err());
    h.insert("x-jx3-provider-settings", "1".parse().unwrap());
    h.insert("origin", "http://127.0.0.1:3039".parse().unwrap());
    h.insert("sec-fetch-site", "same-origin".parse().unwrap());
    assert!(check_headers(&h).is_ok());
    h.insert("sec-fetch-site", "cross-site".parse().unwrap());
    assert!(check_headers(&h).is_err());
    h.insert("sec-fetch-site", "same-origin".parse().unwrap());
    h.insert("origin", "http://public.example.com".parse().unwrap());
    assert_eq!(
        check_headers(&h).unwrap_err().code,
        "custom_secure_page_required"
    );
    h.insert("origin", "https://public.example.com".parse().unwrap());
    assert!(check_headers(&h).is_ok());
}

#[tokio::test]
async fn custom_chat_and_responses_use_real_adapter_tool_transcripts() {
    for protocol in [
        CustomProtocol::ChatCompletions,
        CustomProtocol::Deepseek,
        CustomProtocol::Responses,
    ] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let capture = seen.clone();
        let route = if protocol == CustomProtocol::Responses {
            "/v1/responses"
        } else {
            "/v1/chat/completions"
        };
        let app = Router::new().route(route, post(move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
            let capture = capture.clone();
            async move {
                assert_eq!(headers.get("authorization").unwrap(), &format!("Bearer {TEST_KEY}"));
                let first = { let mut seen = capture.lock().unwrap(); seen.push(body); seen.len() == 1 };
                Json(if protocol == CustomProtocol::Responses {
                    if first { json!({"status":"completed", "output":[{"type":"function_call","call_id":"call_probe","name":"connection_test","arguments":"{\"ok\":true}"}]}) }
                    else { json!({"status":"completed", "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}) }
                } else if first {
                    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_probe","type":"function","function":{"name":"connection_test","arguments":"{\"ok\":true}"}}]}}]})
                } else { json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]}) })
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let f = Fixture::new();
        let provider = f
            .store
            .draft_provider(input(&url, TEST_KEY, protocol))
            .unwrap();
        let mut request = probe_request();
        let first = provider.complete(&request).await.unwrap();
        assert_eq!(first.tool_calls.len(), 1);
        request.messages.push(ModelMessage::Assistant {
            content: None,
            tool_calls: vec![ProviderToolCall {
                call_id: "call_probe".into(),
                name: "connection_test".into(),
                arguments: json!({"ok":true}),
            }],
            reasoning_content: None,
        });
        request.messages.push(ModelMessage::ToolResult {
            call_id: "call_probe".into(),
            output: json!({"ok":true}),
        });
        assert_eq!(
            provider
                .complete(&request)
                .await
                .unwrap()
                .assistant_text
                .as_deref(),
            Some("ok")
        );
        assert!(
            f.store.safe_profile().is_none(),
            "draft testing must not save config"
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["model"], "fixture-model");
        assert!(!seen[0].to_string().contains(TEST_KEY));
        if protocol == CustomProtocol::Responses {
            assert_eq!(seen[0]["store"], false);
        }
        server.abort();
    }
}

#[tokio::test]
async fn custom_endpoint_does_not_follow_redirects_or_dns_to_private_hosts() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            (
                StatusCode::TEMPORARY_REDIRECT,
                [("location", "http://127.0.0.1:1/leak")],
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let f = Fixture::new();
    let p = f
        .store
        .draft_provider(input(
            &format!("http://127.0.0.1:{port}/v1"),
            TEST_KEY,
            CustomProtocol::ChatCompletions,
        ))
        .unwrap();
    let error = p.complete(&probe_request()).await.unwrap_err();
    assert_eq!(error.upstream_status, Some(307));
    // A hostname that resolves to loopback must not inherit local-IP permission.
    let p = f
        .store
        .draft_provider(input(
            &format!("https://localhost.:{port}/v1"),
            TEST_KEY,
            CustomProtocol::ChatCompletions,
        ))
        .unwrap();
    assert_eq!(
        p.complete(&probe_request()).await.unwrap_err().code,
        "provider_network_error"
    );
    server.abort();
}

#[tokio::test]
async fn custom_upstream_cannot_echo_key_into_reports_or_replay() {
    for invalid_arguments in [false, true] {
        let app = Router::new().route("/v1/chat/completions", post(move || async move {
            Json(if invalid_arguments {
                json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_probe","type":"function","function":{"name":"connection_test","arguments":TEST_KEY}}]}}]})
            } else {
                json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":TEST_KEY}}]})
            })
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let f = Fixture::new();
        let provider = f
            .store
            .draft_provider(input(&url, TEST_KEY, CustomProtocol::ChatCompletions))
            .unwrap();
        let error = provider.complete(&probe_request()).await.unwrap_err();
        assert!(error.private_detail.is_none());
        assert!(!format!("{error:?}").contains(TEST_KEY));
        if !invalid_arguments {
            assert_eq!(error.code, "custom_credential_echo");
        }
        server.abort();
    }
}

#[tokio::test]
async fn custom_chunked_upstream_response_is_bounded() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            let chunks = futures_util::stream::iter(
                (0..20).map(|_| Ok::<_, std::convert::Infallible>(vec![b'x'; 64 * 1024])),
            );
            axum::body::Body::from_stream(chunks)
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let f = Fixture::new();
    let provider = f
        .store
        .draft_provider(input(&url, TEST_KEY, CustomProtocol::ChatCompletions))
        .unwrap();
    assert_eq!(
        provider.complete(&probe_request()).await.unwrap_err().code,
        "provider_response_too_large"
    );
    server.abort();
}
