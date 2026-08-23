//! `HttpDir` adapter over the constellation's shared blocking HTTP client.
use sim_kernel::{Error, Result};
use sim_lib_net_http::{
    Cancellation, Client, Header, Method, Policy, Request, RequestBody, TcpConnector, Url,
};
use std::time::Duration;
pub(crate) struct HttpRequest {
    pub(crate) method: &'static str,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) timeout: Duration,
    pub(crate) max_body_bytes: usize,
}
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) reason: Option<String>,
    pub(crate) body: Vec<u8>,
}
pub(crate) fn send(request: HttpRequest) -> Result<HttpResponse> {
    let policy = Policy {
        connect_timeout: request.timeout,
        read_timeout: request.timeout,
        write_timeout: request.timeout,
        total_timeout: request.timeout,
        max_request_bytes: request.body.len().max(1),
        max_response_bytes: request.max_body_bytes,
        max_decompressed_bytes: request.max_body_bytes,
        ..Policy::default()
    };
    let method = Method::new(request.method).map_err(map_error)?;
    let url = Url::parse(request.url).map_err(map_error)?;
    let headers = request
        .headers
        .into_iter()
        .map(|(n, v)| Header::new(n, v).map_err(map_error))
        .collect::<Result<Vec<_>>>()?;
    let response = Client::new(TcpConnector, policy)
        .execute(Request {
            method,
            url,
            headers,
            body: if request.body.is_empty() {
                RequestBody::Empty
            } else {
                RequestBody::Bytes(&request.body)
            },
            deadline: None,
            cancellation: Cancellation::default(),
        })
        .map_err(map_error)?;
    Ok(HttpResponse {
        status: response.status,
        reason: Some(response.reason.clone()),
        body: response.into_body(),
    })
}
fn map_error(error: sim_lib_net_http::Error) -> Error {
    match error {
        sim_lib_net_http::Error::ResponseTooLarge { cap } => Error::Eval(format!(
            "table/http: response exceeded max body bytes {cap}"
        )),
        sim_lib_net_http::Error::InvalidUrl
        | sim_lib_net_http::Error::UserInfoForbidden
        | sim_lib_net_http::Error::UnsupportedScheme
        | sim_lib_net_http::Error::InvalidMethod
        | sim_lib_net_http::Error::InvalidHeaderName
        | sim_lib_net_http::Error::InvalidHeaderValue
        | sim_lib_net_http::Error::AmbiguousHeader
        | sim_lib_net_http::Error::UnsupportedTransferFraming
        | sim_lib_net_http::Error::RequestTooLarge { .. } => {
            Error::Eval(format!("table/http: {error}"))
        }
        _ => Error::HostError(format!("table/http: {error}")),
    }
}
