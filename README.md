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
    Ok(())
}
```

运行环境必须提供 `METIS_PLATFORM_ENDPOINT`、`METIS_APP_ID` 和 `METIS_APP_TOKEN`。SDK 只负责平台 Runtime API 和配置解析，不实现业务协议客户端或自动重试。

## 开发

```bash
cargo fmt --check
cargo test
cargo package --allow-dirty
```

## 许可证

Apache-2.0，见 [LICENSE](LICENSE)。
