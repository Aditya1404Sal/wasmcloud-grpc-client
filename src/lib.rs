// Copyright 2025 Aditya Salunkhe

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at

//     http://www.apache.org/licenses/LICENSE-2.0

// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http::Uri;
use http_body::Body as HttpBody;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use tower_service::Service;

/// Component-side gRPC endpoint that uses wasi:http/outgoing-handler
///
/// This is analogous to wasmcloud_component's helper types - it's a
/// convenience wrapper for components, not part of the host runtime.
#[derive(Clone)]
pub struct GrpcEndpoint {
    endpoint: Uri,
}

impl GrpcEndpoint {
    pub fn new(endpoint: Uri) -> Self {
        Self { endpoint }
    }
}

impl<B> Service<hyper::Request<B>> for GrpcEndpoint
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: std::error::Error + Send + Sync + 'static,
{
    type Response = hyper::Response<Incoming>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<B>) -> Self::Future {
        // Rebuild URI with endpoint authority/scheme
        let endpoint_parts = self.endpoint.clone().into_parts();
        let (mut parts, body) = req.into_parts();
        let mut uri_parts = std::mem::take(&mut parts.uri).into_parts();
        uri_parts.authority = endpoint_parts.authority;
        uri_parts.scheme = endpoint_parts.scheme;
        parts.uri = Uri::from_parts(uri_parts).unwrap();

        // Convert body to bytes
        Box::pin(async move {
            let body_bytes = body
                .collect()
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                .to_bytes();

            // Use wasi:http/outgoing-handler to send the request
            // This calls into the host's HttpClient plugin
            let wasi_request = wasmcloud_component::wasi::http::types::OutgoingRequest::new(
                wasmcloud_component::wasi::http::types::Headers::new(),
            );
            // TODO: Set method, URI, headers, body on wasi_request

            let wasi_response =
                wasmcloud_component::wasi::http::outgoing_handler::handle(wasi_request, None)?;

            // TODO: Convert wasi_response back to hyper::Response<Incoming>

            unimplemented!("WASI type conversion needed")
        })
    }
}
