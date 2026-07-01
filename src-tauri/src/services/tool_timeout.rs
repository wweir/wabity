use std::{future::Future, time::Duration};

use anyhow::{bail, Result};

pub(crate) async fn execute_with_timeout<T, F>(
    tool_name: &str,
    timeout_duration: Duration,
    future: F,
) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout_duration, future).await {
        Ok(result) => result,
        Err(_) => bail!(
            "内置工具 {} 执行超时（>{} ms）",
            tool_name,
            timeout_duration.as_millis()
        ),
    }
}
