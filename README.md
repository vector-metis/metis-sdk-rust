# Metis Rust SDK

Metis 应用后端 SDK，用于读取统一应用身份、可信上下文、声明的依赖，以及模型和对象存储配置。

## 安装

```bash
cargo add metis-platform-sdk
```

```rust
use metis_platform_sdk::Client;

fn main() -> Result<(), metis_platform_sdk::MetisError> {
    let client = Client::from_env()?;
    println!("{}", client.application().id);
    let embedding = client.model("embedding.0")?;
    let rerank = client.model("rerank.0")?;
    println!("{} {}", embedding.model, rerank.model);
    Ok(())
}
```

`model()` 支持 `llm.N`、`embedding.N` 和 `rerank.N` 三类 slot，并返回对应的网关地址、模型别名、API key 及类型专属参数。SDK 不创建厂商客户端。

运行环境必须提供 `METIS_PLATFORM_ENDPOINT`、`METIS_APP_ID` 和 `METIS_APP_TOKEN`。SDK 只负责平台 Runtime API 和配置解析，不实现业务协议客户端或自动重试。

`Dependency` 同时提供 `requested_version`、`resolved_version`、`package_sha256`、`direct`、
`available` 和 `resolution_error`，用于检查平台锁定版本及可选依赖的降级状态。

## 开发

```bash
cargo fmt --check
cargo test
cargo package --allow-dirty
```

## 许可证

Apache-2.0，见 [LICENSE](LICENSE)。
