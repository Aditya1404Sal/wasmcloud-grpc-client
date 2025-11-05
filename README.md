# NOTE : This is a WIP, DO NOT USE.

# wasmcloud-grpc-client

Enables **gRPC clients** to work inside **wasmCloud** components using the standard `wasi:http/outgoing-handler` interface.

wasmCloud is a fast, secure WebAssembly framework for distributed applications. This project extends wasmCloud's capabilities by enabling components to make outbound gRPC requests, bridging the gap between Wasm sandboxing and modern microservice communication.

## Features

-  **gRPC over HTTP/2** support inside wasmCloud components
-  Compatible with the wasmCloud security model via WASI interfaces
-  Built with `tonic` and the standard `wasi:http` interface
-  Works with most standard gRPC services
-  Great for calling internal microservices or public gRPC APIs from wasmCloud components
-  Automatic HTTP/2 connection pooling via the wasmCloud runtime

## Usage

### 1. Add required dependencies to your Cargo.toml

```toml
[dependencies]
anyhow = "1"
wasmcloud-component = "0.x"  # wasmCloud component SDK
wasmcloud-grpc-client = "0.1.0"
tonic = { version = "0.12", default-features = false }
prost = "0.13"

[build-dependencies]
tonic-build = { version = "0.12", features = ["prost"] }
```

### 2. Generate gRPC client code with `tonic-build`

In `build.rs`:

```rust
fn main() {
    tonic_build::configure()
        .build_transport(false)  // Important: disable default transport
        .compile_protos(&["proto/helloworld.proto"], &["proto"])
        .unwrap();
}
```

### 3. Configure your wasmCloud component manifest

In your `wasmcloud.toml`:

```toml
name = "my-grpc-client"
language = "rust"
type = "component"

[component]
# Grant permission to make outbound HTTP requests
wit_world = "http-client"
```

### 4. Call your gRPC service from a wasmCloud component

```rust
use anyhow::Context;
use tonic::Request;
use wasmcloud_grpc_client::GrpcEndpoint;

use hello_world::greeter_client::GreeterClient;
use hello_world::HelloRequest;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

struct Component;

impl wasmcloud_component::http::Server for Component {
    fn handle(
        _req: wasmcloud_component::http::IncomingRequest,
    ) -> wasmcloud_component::http::Result<
        wasmcloud_component::http::Response<impl wasmcloud_component::http::OutgoingBody>
    > {
        // Parse the gRPC endpoint URI
        let endpoint_uri = "http://[::1]:50051"
            .parse()
            .context("Failed to parse endpoint URI")?;
        
        // Create the gRPC endpoint wrapper
        let endpoint = GrpcEndpoint::new(endpoint_uri);
        
        // Create the gRPC client
        let mut client = GreeterClient::new(endpoint);

        // Make the gRPC call
        let request = Request::new(HelloRequest {
            name: "wasmCloud".to_string(),
        });

        let response = client
            .say_hello(request)
            .await
            .context("gRPC call failed")?;

        let message = response.into_inner().message;

        Ok(wasmcloud_component::http::Response::new(message))
    }
}

wasmcloud_component::http::export!(Component);
```

## How It Works

```
┌─────────────────────────────────────┐
│   Your Component (WebAssembly)     │
│                                     │
│   GreeterClient::say_hello()        │
└──────────────┬──────────────────────┘
               │ (tonic generates this)
               ↓
┌─────────────────────────────────────┐
│   GrpcEndpoint (Tower Service)      │
│   - Converts hyper → WASI types     │
│   - Calls wasi:http/outgoing-handler│
└──────────────┬──────────────────────┘
               │ (WASI interface boundary)
               ↓
┌─────────────────────────────────────┐
│   wasmCloud Runtime (Host)          │
│   - HTTP/2 connection pooling       │
│   - TLS with ALPN negotiation       │
│   - Actual network I/O              │
└─────────────────────────────────────┘
```

The `GrpcEndpoint` acts as a bridge between tonic's hyper-based transport and wasmCloud's `wasi:http/outgoing-handler` interface. This allows gRPC clients to work seamlessly inside WebAssembly components while the wasmCloud runtime handles connection management, HTTP/2 multiplexing, and TLS.

## Security Model

wasmCloud components run in a secure sandbox with capability-based security. To make outbound gRPC requests:

1. Your component must import the `wasi:http/outgoing-handler` interface
2. The wasmCloud runtime enforces network access policies
3. Connection pooling and TLS are handled securely by the host

This provides strong isolation between components while enabling controlled access to external services.

## Connection Pooling

The wasmCloud runtime automatically pools HTTP/2 connections for you:

- **Automatic multiplexing**: Multiple gRPC calls reuse the same connection
- **Transparent to components**: No manual connection management needed
- **Efficient resource usage**: Connections are pooled per endpoint
- **ALPN negotiation**: Automatically selects HTTP/2 for HTTPS endpoints

## TLS Support

For secure gRPC (gRPC over HTTPS):

```rust
// Just use https:// in the URI - TLS is handled automatically
let endpoint_uri = "https://grpc.example.com:443".parse()?;
let endpoint = GrpcEndpoint::new(endpoint_uri);
let mut client = MyServiceClient::new(endpoint);
```

The wasmCloud runtime handles:
- TLS handshake and certificate verification
- ALPN negotiation (prefers HTTP/2)
- Connection security

## Comparison with Spin

| Feature | wasmCloud | Spin |
|---------|-----------|------|
| **Interface** | `wasi:http/outgoing-handler` | Custom Spin SDK |
| **Runtime** | wasmCloud (lattice-capable) | Spin runtime |
| **Connection Pooling** | Automatic (HTTP/2) | Automatic (HTTP/2) |
| **TLS** | Built-in with ALPN | Built-in with ALPN |
| **Standards** | WASI standard interfaces | Spin-specific APIs |

## 📚 Examples

TODO

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## 📄 License

This project is licensed under the Apache License 2.0 - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Inspired by [`wasi-grpc`](https://github.com/fermyon/wasi-grpc) from Fermyon
- Built on [`tonic`](https://github.com/hyperium/tonic) for gRPC client generation
- Uses the standard [`wasi:http`](https://github.com/WebAssembly/wasi-http) interface