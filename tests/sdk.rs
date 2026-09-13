use metis_platform_sdk::{context_from_headers, Client, Config, MetisError};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn fixture_environment() -> HashMap<String, String> {
    [
        ("METIS_PLATFORM_ENDPOINT", "http://127.0.0.1"),
        ("METIS_APP_ID", "caller-a7x2m"),
        ("METIS_APP_TOKEN", "app-token"),
        (
            "METIS_LLM_0_ENDPOINT",
            "http://platform.example.invalid/api/model-gateway",
        ),
        ("METIS_LLM_0_MODEL", "example-chat"),
        ("METIS_LLM_0_API_KEY", "model-token"),
        (
            "METIS_EMBEDDING_0_ENDPOINT",
            "http://platform.example.invalid/api/model-gateway",
        ),
        ("METIS_EMBEDDING_0_MODEL", "example-embedding"),
        ("METIS_EMBEDDING_0_API_KEY", "embedding-token"),
        ("METIS_EMBEDDING_0_DIMENSIONS", "1024"),
        ("METIS_EMBEDDING_0_NORMALIZED", "true"),
        (
            "METIS_RERANK_0_ENDPOINT",
            "http://platform.example.invalid/api/model-gateway",
        ),
        ("METIS_RERANK_0_MODEL", "example-rerank"),
        ("METIS_RERANK_0_API_KEY", "rerank-token"),
        ("METIS_RERANK_0_MAX_DOCUMENTS", "64"),
        ("METIS_S3_ENDPOINT", "http://storage.example.invalid"),
        ("METIS_S3_ACCESS_KEY", "app-access"),
        ("METIS_S3_SECRET_KEY", "app-secret"),
        ("METIS_S3_BUCKET", "caller-a7x2m"),
        ("METIS_S3_SHARED_BUCKETS", "[\"shared-assets\"]"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

#[test]
fn shared_contract_cache_and_config() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let thread_calls = calls.clone();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let size = stream.read(&mut request).unwrap();
            let request_text = String::from_utf8_lossy(&request[..size]);
            assert!(request_text
                .to_ascii_lowercase()
                .contains("authorization: bearer app-token"));
            thread_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let body = if request_text.contains("/endpoints/database") {
                r#"{"appId":"service-dependency-a7x2m","endpointName":"database","protocol":"RUNTIME_ENDPOINT_PROTOCOL_TCP","host":"127.0.0.1","port":31001,"available":true}"#
            } else {
                r#"{"dependencies":[{"appId":"web-dependency-a7x2m","alias":"ui","required":true,"requestedVersion":"^1.2.0","resolvedVersion":"1.4.2","packageSha256":"sha256:web","available":true,"direct":true,"resolutionError":"","appType":"RUNTIME_APPLICATION_TYPE_WEB","webBasePath":"/apps/web-dependency-a7x2m"}]}"#
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    let mut environment = fixture_environment();
    environment.insert(
        "METIS_PLATFORM_ENDPOINT".into(),
        format!("http://{address}"),
    );
    let client = Client::new(Config {
        environment: Some(environment),
        ..Config::default()
    })
    .unwrap();
    assert_eq!(client.list_dependencies(false).unwrap()[0].alias, "ui");
    let dependency = client.list_dependencies(false).unwrap().remove(0);
    assert_eq!(dependency.requested_version, "^1.2.0");
    assert_eq!(dependency.resolved_version, "1.4.2");
    assert_eq!(dependency.package_sha256, "sha256:web");
    assert!(dependency.direct);
    assert_eq!(client.list_dependencies(false).unwrap()[0].alias, "ui");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        client
            .service_endpoint("data", "database", true)
            .unwrap()
            .port,
        31001
    );
    assert_eq!(client.model("llm.0").unwrap().model, "example-chat");
    assert_eq!(
        client.model("embedding.0").unwrap().model,
        "example-embedding"
    );
    assert_eq!(
        client.model("embedding.0").unwrap().values["DIMENSIONS"],
        "1024"
    );
    assert_eq!(client.model("rerank.0").unwrap().model, "example-rerank");
    assert_eq!(
        client.model("rerank.0").unwrap().values["MAX_DOCUMENTS"],
        "64"
    );
    assert_eq!(
        client.object_storage().unwrap().shared_buckets,
        vec!["shared-assets"]
    );
    let mut headers = HashMap::new();
    headers.insert("x-platform-tenant-id".into(), "42".into());
    assert_eq!(context_from_headers(&headers).tenant_id, "42");
    server.join().unwrap();
}

#[test]
fn invalid_values_use_stable_errors() {
    let mut environment = fixture_environment();
    environment.insert("METIS_S3_SHARED_BUCKETS".into(), "null".into());
    let client = Client::new(Config {
        platform_endpoint: Some("http://localhost".into()),
        app_id: Some("local-app".into()),
        app_token: Some("token".into()),
        environment: Some(environment),
        ..Config::default()
    })
    .unwrap();
    assert!(matches!(
        client.model("llm.-1"),
        Err(MetisError::InvalidConfig(_))
    ));
    assert!(matches!(
        client.object_storage(),
        Err(MetisError::InvalidConfig(_))
    ));
    assert!(matches!(
        client.web_url("ui", "../admin"),
        Err(MetisError::Transport(_)) | Err(MetisError::Upstream { .. })
    ));
}
