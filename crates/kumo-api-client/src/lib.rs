use anyhow::Context;
use futures::{Stream, StreamExt};
use kumo_api_types::rebind::{RebindV1Request, RebindV1Response};
use kumo_api_types::xfer::*;
use kumo_api_types::*;
use kumo_prometheus::parser::Metric;
use std::time::Duration;

pub use reqwest::Url;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct KumoApiClient {
    endpoint: Url,
    timeout: Duration,
}

macro_rules! method {
    ($func_name:ident, POST, $path:literal, $request_ty:ty, $response_ty:ty) => {
        pub async fn $func_name(&self, params: &$request_ty) -> anyhow::Result<$response_ty> {
            self.request_with_json_response(
                reqwest::Method::POST,
                self.endpoint.join($path)?,
                params,
            )
            .await
        }
    };

    ($func_name:ident, TEXT, DELETE, $path:literal, $request_ty:ty) => {
        pub async fn $func_name(&self, params: &$request_ty) -> anyhow::Result<String> {
            self.request_with_text_response(
                reqwest::Method::DELETE,
                self.endpoint.join($path)?,
                params,
            )
            .await
        }
    };

    ($func_name:ident, TEXT, POST, $path:literal, $request_ty:ty) => {
        pub async fn $func_name(&self, params: &$request_ty) -> anyhow::Result<String> {
            self.request_with_text_response(
                reqwest::Method::POST,
                self.endpoint.join($path)?,
                params,
            )
            .await
        }
    };

    ($func_name:ident, GET, $path:literal, $request_ty:ty, $response_ty:ty) => {
        pub async fn $func_name(&self, get_params: &$request_ty) -> anyhow::Result<$response_ty> {
            let mut url = self.endpoint.join($path)?;
            get_params.apply_to_url(&mut url);

            self.request_with_json_response(reqwest::Method::GET, url, &())
                .await
        }
    };

    ($func_name:ident, GET, $path:literal, $response_ty:ty) => {
        pub async fn $func_name(&self) -> anyhow::Result<$response_ty> {
            self.request_with_json_response(reqwest::Method::GET, self.endpoint.join($path)?, &())
                .await
        }
    };
}

impl KumoApiClient {
    pub fn new(endpoint: Url) -> Self {
        Self {
            endpoint,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    fn client_builder(&self) -> reqwest::ClientBuilder {
        reqwest::Client::builder().timeout(self.timeout)
    }

    method!(
        admin_bounce_v1,
        POST,
        "/api/admin/bounce/v1",
        BounceV1Request,
        BounceV1Response
    );

    method!(machine_info, GET, "/api/machine-info", MachineInfoV1);

    method!(
        admin_bounce_list_v1,
        GET,
        "/api/admin/bounce/v1",
        Vec<BounceV1ListEntry>
    );

    method!(
        admin_bounce_cancel_v1,
        TEXT,
        DELETE,
        "/api/admin/bounce/v1",
        BounceV1CancelRequest
    );

    method!(
        admin_inspect_sched_q_v1,
        GET,
        "/api/admin/inspect-sched-q/v1",
        InspectQueueV1Request,
        InspectQueueV1Response
    );

    method!(
        admin_inspect_message_v1,
        GET,
        "/api/admin/inspect-message/v1",
        InspectMessageV1Request,
        InspectMessageV1Response
    );

    method!(
        admin_xfer_v1,
        POST,
        "/api/admin/xfer/v1",
        XferV1Request,
        XferV1Response
    );

    method!(
        admin_suspend_list_v1,
        GET,
        "/api/admin/suspend/v1",
        Vec<SuspendV1ListEntry>
    );

    method!(
        admin_suspend_ready_q_list_v1,
        GET,
        "/api/admin/suspend-ready-q/v1",
        Vec<SuspendReadyQueueV1ListEntry>
    );

    method!(
        admin_xfer_cancel_v1,
        POST,
        "/api/admin/xfer/cancel/v1",
        XferCancelV1Request,
        XferCancelV1Response
    );

    method!(
        admin_rebind_v1,
        POST,
        "/api/admin/rebind/v1",
        RebindV1Request,
        RebindV1Response
    );

    method!(
        admin_suspend_ready_q_v1,
        POST,
        "/api/admin/suspend-ready-q/v1",
        SuspendReadyQueueV1Request,
        SuspendV1Response
    );

    method!(
        admin_suspend_ready_q_cancel_v1,
        TEXT,
        DELETE,
        "/api/admin/suspend-ready-q/v1",
        SuspendV1CancelRequest
    );

    method!(
        admin_suspend_v1,
        POST,
        "/api/admin/suspend/v1",
        SuspendV1Request,
        SuspendV1Response
    );

    method!(
        admin_suspend_cancel_v1,
        TEXT,
        DELETE,
        "/api/admin/suspend/v1",
        SuspendV1CancelRequest
    );

    method!(
        admin_ready_q_states_v1,
        GET,
        "/api/admin/ready-q-states/v1",
        ReadyQueueStateRequest,
        ReadyQueueStateResponse
    );

    method!(
        admin_set_diagnostic_log_filter_v1,
        TEXT,
        POST,
        "/api/admin/set_diagnostic_log_filter/v1",
        SetDiagnosticFilterRequest
    );

    pub async fn request_with_text_response<T: reqwest::IntoUrl, B: serde::Serialize>(
        &self,
        method: reqwest::Method,
        url: T,
        body: &B,
    ) -> anyhow::Result<String> {
        let response = self
            .client_builder()
            .build()?
            .request(method, url)
            .json(body)
            .send()
            .await?;

        let status = response.status();
        let body_bytes = response.bytes().await.with_context(|| {
            format!(
                "request status {}: {}, and failed to read response body",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )
        })?;
        let body_text = String::from_utf8_lossy(&body_bytes);
        if !status.is_success() {
            anyhow::bail!(
                "request status {}: {}. Response body: {body_text}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
            );
        }

        Ok(body_text.to_string())
    }

    pub async fn admin_bump_config_epoch(&self) -> anyhow::Result<String> {
        self.request_with_text_response(
            reqwest::Method::POST,
            self.endpoint.join("/api/admin/bump-config-epoch")?,
            &(),
        )
        .await
    }

    pub async fn request_with_streaming_text_response<T: reqwest::IntoUrl, B: serde::Serialize>(
        &self,
        method: reqwest::Method,
        url: T,
        body: &B,
    ) -> anyhow::Result<impl Stream<Item = reqwest::Result<bytes::Bytes>>> {
        let response = self
            .client_builder()
            .build()?
            .request(method, url)
            .json(body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body_bytes = response.bytes().await.with_context(|| {
                format!(
                    "request status {}: {}, and failed to read response body",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )
            })?;
            let body_text = String::from_utf8_lossy(&body_bytes);
            anyhow::bail!(
                "request status {}: {}. Response body: {body_text}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
            );
        }

        Ok(response.bytes_stream())
    }

    pub async fn request_with_json_response<
        T: reqwest::IntoUrl,
        B: serde::Serialize,
        R: serde::de::DeserializeOwned,
    >(
        &self,
        method: reqwest::Method,
        url: T,
        body: &B,
    ) -> anyhow::Result<R> {
        let response = self
            .client_builder()
            .build()?
            .request(method, url)
            .json(body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body_bytes = response.bytes().await.with_context(|| {
                format!(
                    "request status {}: {}, and failed to read response body",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )
            })?;
            anyhow::bail!(
                "request status {}: {}. Response body: {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
                String::from_utf8_lossy(&body_bytes)
            );
        }
        json_body(response).await.with_context(|| {
            format!(
                "request status {}: {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )
        })
    }

    pub async fn get_metrics<T, F: FnMut(&Metric) -> Option<T>>(
        &self,
        mut filter_map: F,
    ) -> anyhow::Result<Vec<T>> {
        let mut parser = kumo_prometheus::parser::Parser::new();
        let mut stream = self
            .request_with_streaming_text_response(
                reqwest::Method::GET,
                self.endpoint.join("/metrics")?,
                &(),
            )
            .await?;

        let mut result = vec![];
        while let Some(item) = stream.next().await {
            let bytes = item?;
            parser.push_bytes(bytes, false, |m| {
                if let Some(r) = (filter_map)(&m) {
                    result.push(r);
                }
            })?;
        }

        Ok(result)
    }
}

pub async fn json_body<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let data = response.bytes().await.context("ready response body")?;
    serde_json::from_slice(&data).with_context(|| {
        format!(
            "parsing response as json: {}",
            String::from_utf8_lossy(&data)
        )
    })
}
