use std::io::Read;

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::{parsing::parse_response_payload, streaming::StreamingPayloadCollector};

const STREAM_READ_BUFFER_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiCompatibleResponseFormat {
    Json,
    JsonOrSse,
}

pub struct OpenAiCompatibleClient<'a, HttpClient> {
    client: &'a HttpClient,
    base_url: String,
    api_key: &'a str,
}

pub fn normalize_base_url(base_url: &str, label: &str) -> Result<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    if normalized.is_empty() {
        anyhow::bail!("{label} 不能为空");
    }

    reqwest::Url::parse(normalized).with_context(|| format!("invalid {label}: {normalized}"))?;
    Ok(normalized.to_string())
}

impl<'a> OpenAiCompatibleClient<'a, reqwest::Client> {
    pub fn new_async(
        client: &'a reqwest::Client,
        base_url: &'a str,
        api_key: &'a str,
        base_url_label: &str,
    ) -> Result<Self> {
        Ok(Self {
            client,
            base_url: normalize_base_url(base_url, base_url_label)?,
            api_key: api_key.trim(),
        })
    }

    pub async fn post_json<T, B>(
        &self,
        endpoint: &str,
        body: &B,
        operation: &str,
        response_format: OpenAiCompatibleResponseFormat,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let mut request =
            self.client
                .post(format!("{}{}", self.base_url, normalize_endpoint(endpoint)));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(self.api_key);
        }

        let response = request
            .json(body)
            .send()
            .await
            .with_context(|| format!("failed to request {operation}"))?;
        let payload =
            read_async_response_payload(response, endpoint, operation, response_format).await?;
        serde_json::from_value(payload)
            .with_context(|| format!("failed to deserialize {operation} response"))
    }

    pub async fn post_json_with_text_stream<T, B, F>(
        &self,
        endpoint: &str,
        body: &B,
        operation: &str,
        response_format: OpenAiCompatibleResponseFormat,
        on_text_delta: F,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
        F: FnMut(&str),
    {
        let mut request =
            self.client
                .post(format!("{}{}", self.base_url, normalize_endpoint(endpoint)));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(self.api_key);
        }

        let response = request
            .json(body)
            .send()
            .await
            .with_context(|| format!("failed to request {operation}"))?;
        let payload = read_async_response_payload_with_text_stream(
            response,
            endpoint,
            operation,
            response_format,
            on_text_delta,
        )
        .await?;
        serde_json::from_value(payload)
            .with_context(|| format!("failed to deserialize {operation} response"))
    }
}

impl<'a> OpenAiCompatibleClient<'a, reqwest::blocking::Client> {
    pub fn new_blocking(
        client: &'a reqwest::blocking::Client,
        base_url: &'a str,
        api_key: &'a str,
        base_url_label: &str,
    ) -> Result<Self> {
        Ok(Self {
            client,
            base_url: normalize_base_url(base_url, base_url_label)?,
            api_key: api_key.trim(),
        })
    }

    pub fn post_json<T, B>(
        &self,
        endpoint: &str,
        body: &B,
        operation: &str,
        response_format: OpenAiCompatibleResponseFormat,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let mut request =
            self.client
                .post(format!("{}{}", self.base_url, normalize_endpoint(endpoint)));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(self.api_key);
        }

        let response = request
            .json(body)
            .send()
            .with_context(|| format!("failed to request {operation}"))?;
        let payload =
            read_blocking_response_payload(response, endpoint, operation, response_format)?;
        serde_json::from_value(payload)
            .with_context(|| format!("failed to deserialize {operation} response"))
    }

    pub fn get_json<T>(&self, endpoint: &str, operation: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let mut request =
            self.client
                .get(format!("{}{}", self.base_url, normalize_endpoint(endpoint)));
        if !self.api_key.is_empty() {
            request = request.bearer_auth(self.api_key);
        }

        let response = request
            .send()
            .with_context(|| format!("failed to request {operation}"))?;
        let status = response.status();
        let body = response
            .text()
            .with_context(|| format!("failed to read {operation} response body"))?;
        let payload = parse_response_payload(
            status,
            body,
            endpoint,
            operation,
            OpenAiCompatibleResponseFormat::Json,
        )?;
        serde_json::from_value(payload)
            .with_context(|| format!("failed to deserialize {operation} response"))
    }
}

fn normalize_endpoint(endpoint: &str) -> String {
    if endpoint.starts_with('/') {
        endpoint.to_string()
    } else {
        format!("/{endpoint}")
    }
}

async fn read_async_response_payload(
    mut response: reqwest::Response,
    endpoint: &str,
    operation: &str,
    response_format: OpenAiCompatibleResponseFormat,
) -> Result<Value> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .with_context(|| format!("failed to read {operation} response body"))?;
        return parse_response_payload(status, body, endpoint, operation, response_format);
    }

    match response_format {
        OpenAiCompatibleResponseFormat::Json => {
            let body = response
                .text()
                .await
                .with_context(|| format!("failed to read {operation} response body"))?;
            parse_response_payload(status, body, endpoint, operation, response_format)
        }
        OpenAiCompatibleResponseFormat::JsonOrSse => {
            let mut collector = StreamingPayloadCollector::default();
            while let Some(chunk) = response
                .chunk()
                .await
                .with_context(|| format!("failed to read {operation} response stream"))?
            {
                collector.push_bytes(&chunk)?;
            }

            collector.finish(&format!("{operation} response"))
        }
    }
}

async fn read_async_response_payload_with_text_stream<F>(
    mut response: reqwest::Response,
    endpoint: &str,
    operation: &str,
    response_format: OpenAiCompatibleResponseFormat,
    mut on_text_delta: F,
) -> Result<Value>
where
    F: FnMut(&str),
{
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .with_context(|| format!("failed to read {operation} response body"))?;
        return parse_response_payload(status, body, endpoint, operation, response_format);
    }

    match response_format {
        OpenAiCompatibleResponseFormat::Json => {
            let body = response
                .text()
                .await
                .with_context(|| format!("failed to read {operation} response body"))?;
            parse_response_payload(status, body, endpoint, operation, response_format)
        }
        OpenAiCompatibleResponseFormat::JsonOrSse => {
            let mut collector = StreamingPayloadCollector::default();
            while let Some(chunk) = response
                .chunk()
                .await
                .with_context(|| format!("failed to read {operation} response stream"))?
            {
                collector.push_bytes_with_text_stream(&chunk, &mut on_text_delta)?;
            }

            collector.finish(&format!("{operation} response"))
        }
    }
}

fn read_blocking_response_payload(
    mut response: reqwest::blocking::Response,
    endpoint: &str,
    operation: &str,
    response_format: OpenAiCompatibleResponseFormat,
) -> Result<Value> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .with_context(|| format!("failed to read {operation} response body"))?;
        return parse_response_payload(status, body, endpoint, operation, response_format);
    }

    match response_format {
        OpenAiCompatibleResponseFormat::Json => {
            let body = response
                .text()
                .with_context(|| format!("failed to read {operation} response body"))?;
            parse_response_payload(status, body, endpoint, operation, response_format)
        }
        OpenAiCompatibleResponseFormat::JsonOrSse => {
            let mut collector = StreamingPayloadCollector::default();
            let mut buffer = [0_u8; STREAM_READ_BUFFER_SIZE];
            loop {
                let bytes_read = response
                    .read(&mut buffer)
                    .with_context(|| format!("failed to read {operation} response stream"))?;
                if bytes_read == 0 {
                    break;
                }

                collector.push_bytes(&buffer[..bytes_read])?;
            }

            collector.finish(&format!("{operation} response"))
        }
    }
}
