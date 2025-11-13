use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http::Uri;
use http_body::Body as HttpBody;
use http_body_util::BodyExt;
use tower_service::Service;
use tracing::{debug, warn};

/// Component-side gRPC endpoint that uses wasi:http/outgoing-handler
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
    type Response = hyper::Response<WasiResponseBody>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<B>) -> Self::Future {
        use wasmcloud_component::wasi::http::{outgoing_handler, types};

        let endpoint_parts = self.endpoint.clone().into_parts();
        let (mut parts, body) = req.into_parts();
        let mut uri_parts = std::mem::take(&mut parts.uri).into_parts();
        uri_parts.authority = endpoint_parts.authority;
        uri_parts.scheme = endpoint_parts.scheme;

        let final_uri = Uri::from_parts(uri_parts);

        Box::pin(async move {
            let final_uri =
                final_uri.map_err(|e| format!("failed to construct request URI: {e}"))?;
            parts.uri = final_uri;

            debug!(
                method = %parts.method,
                uri = %parts.uri,
                "sending gRPC request via WASI"
            );

            let body_bytes = body
                .collect()
                .await
                .map_err(|e| format!("failed to collect request body: {e}"))?
                .to_bytes();

            let headers = types::Fields::new();

            // Skip HTTP/2 pseudo-headers and HTTP/1.1 connection-specific headers
            for (name, value) in parts.headers.iter() {
                let name_str = name.as_str();

                if name_str.starts_with(':') {
                    debug!(header = name_str, "skipping pseudo-header");
                    continue;
                }

                match name_str.to_lowercase().as_str() {
                    "connection" | "keep-alive" | "proxy-connection" | "transfer-encoding"
                    | "upgrade" => {
                        debug!(header = name_str, "skipping HTTP/1.1 connection header");
                        continue;
                    }
                    "te" => {
                        debug!("Skipping Forbidden header");
                        continue;
                    }
                    _ => {}
                }

                let value_bytes = value.as_bytes().to_vec();
                headers
                    .append(&name_str.to_string(), &value_bytes)
                    .map_err(|e| format!("failed to append header {name_str}: {e:?}"))?;
            }

            let wasi_request = types::OutgoingRequest::new(headers);

            let method = convert_method(&parts.method);
            wasi_request
                .set_method(&method)
                .map_err(|e| format!("failed to set HTTP method: {e:?}"))?;

            if let Some(scheme) = parts.uri.scheme() {
                let wasi_scheme = convert_scheme(scheme);
                wasi_request
                    .set_scheme(Some(&wasi_scheme))
                    .map_err(|e| format!("failed to set URI scheme: {e:?}"))?;
            }

            if let Some(authority) = parts.uri.authority() {
                wasi_request
                    .set_authority(Some(authority.as_str()))
                    .map_err(|e| format!("failed to set URI authority: {e:?}"))?;
            }

            if let Some(path_and_query) = parts.uri.path_and_query() {
                wasi_request
                    .set_path_with_query(Some(path_and_query.as_str()))
                    .map_err(|e| format!("failed to set URI path: {e:?}"))?;
            }

            let outgoing_body = wasi_request
                .body()
                .map_err(|e| format!("failed to get request body: {e:?}"))?;

            let output_stream = outgoing_body
                .write()
                .map_err(|e| format!("failed to get body output stream: {e:?}"))?;

            output_stream
                .blocking_write_and_flush(&body_bytes)
                .map_err(|e| format!("failed to write request body: {e:?}"))?;

            drop(output_stream);
            types::OutgoingBody::finish(outgoing_body, None)
                .map_err(|e| format!("failed to finish request body: {e:?}"))?;

            let request_options = types::RequestOptions::new();
            let future_response = outgoing_handler::handle(wasi_request, Some(request_options))
                .map_err(|e| format!("failed to initiate HTTP request: {e:?}"))?;

            future_response.subscribe().block();

            let incoming_response = match future_response.get() {
                Some(Ok(Ok(resp))) => resp,
                Some(Ok(Err(e))) => return Err(format!("request failed: {e:?}").into()),
                Some(Err(_)) => return Err("response future error".into()),
                None => return Err("response future not ready".into()),
            };

            debug!(
                status = incoming_response.status(),
                "received gRPC response"
            );

            let status_code = incoming_response.status();
            let mut response_builder = hyper::Response::builder().status(status_code);

            let response_headers = incoming_response.headers();
            for (name, value) in response_headers.entries() {
                response_builder = response_builder.header(name.as_str(), value.as_slice());
            }

            let response_body = incoming_response
                .consume()
                .map_err(|e| format!("failed to consume response: {e:?}"))?;

            let input_stream = response_body
                .stream()
                .map_err(|e| format!("failed to get response stream: {e:?}"))?;

            let body = WasiResponseBody {
                input_stream,
                _response_body: response_body,
            };
            response_builder
                .body(body)
                .map_err(|e| format!("failed to build response: {e}").into())
        })
    }
}

/// Response body that streams from WASI HTTP input stream
pub struct WasiResponseBody {
    input_stream: wasmcloud_component::wasi::io::streams::InputStream,
    _response_body: wasmcloud_component::wasi::http::types::IncomingBody,
}

impl HttpBody for WasiResponseBody {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        match self.input_stream.blocking_read(8192) {
            Ok(chunk) if chunk.is_empty() => Poll::Ready(None),
            Ok(chunk) => Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(chunk))))),
            Err(wasmcloud_component::wasi::io::streams::StreamError::Closed) => Poll::Ready(None),
            Err(e) => {
                warn!(error = ?e, "failed to read from response stream");
                Poll::Ready(Some(Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("stream read error: {e:?}"),
                ))
                    as Box<dyn std::error::Error + Send + Sync>)))
            }
        }
    }
}

fn convert_method(method: &hyper::Method) -> wasmcloud_component::wasi::http::types::Method {
    use hyper::Method as HyperMethod;
    use wasmcloud_component::wasi::http::types::Method as WasiMethod;

    match *method {
        HyperMethod::GET => WasiMethod::Get,
        HyperMethod::POST => WasiMethod::Post,
        HyperMethod::PUT => WasiMethod::Put,
        HyperMethod::DELETE => WasiMethod::Delete,
        HyperMethod::HEAD => WasiMethod::Head,
        HyperMethod::OPTIONS => WasiMethod::Options,
        HyperMethod::CONNECT => WasiMethod::Connect,
        HyperMethod::PATCH => WasiMethod::Patch,
        HyperMethod::TRACE => WasiMethod::Trace,
        _ => WasiMethod::Other(method.as_str().to_string()),
    }
}

fn convert_scheme(
    scheme: &hyper::http::uri::Scheme,
) -> wasmcloud_component::wasi::http::types::Scheme {
    use wasmcloud_component::wasi::http::types::Scheme as WasiScheme;

    if scheme == &hyper::http::uri::Scheme::HTTPS {
        WasiScheme::Https
    } else if scheme == &hyper::http::uri::Scheme::HTTP {
        WasiScheme::Http
    } else {
        WasiScheme::Other(scheme.as_str().to_string())
    }
}
