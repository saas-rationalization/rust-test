use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EthError {
    #[error("http error: {0}")]
    Http(String),
    #[error("rpc error: {0}")]
    Rpc(String),
    #[error("invalid response: {0}")]
    Response(String),
}

#[derive(Debug, Clone)]
pub struct EthClient {
    rpc_url: String,
}

impl EthClient {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
        }
    }

    pub fn default_anvil() -> Self {
        Self::new("http://127.0.0.1:8545")
    }

    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    pub fn chain_id(&self) -> Result<u64, EthError> {
        let value = self.call("eth_chainId", serde_json::json!([]))?;
        parse_hex_u64(&value)
    }

    pub fn block_number(&self) -> Result<u64, EthError> {
        let value = self.call("eth_blockNumber", serde_json::json!([]))?;
        parse_hex_u64(&value)
    }

    pub fn balance_wei(&self, address: &str) -> Result<String, EthError> {
        let value = self.call("eth_getBalance", serde_json::json!([address, "latest"]))?;
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| EthError::Response("balance not a string".into()))
    }

    pub fn ping(&self) -> Result<bool, EthError> {
        let _ = self.chain_id()?;
        Ok(true)
    }

    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, EthError> {
        #[derive(Serialize)]
        struct RpcRequest<'a> {
            jsonrpc: &'a str,
            id: u64,
            method: &'a str,
            params: serde_json::Value,
        }

        #[derive(Deserialize)]
        struct RpcResponse {
            result: Option<serde_json::Value>,
            error: Option<RpcErrorObject>,
        }

        #[derive(Deserialize)]
        struct RpcErrorObject {
            message: String,
        }

        let body = RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        };

        let body_str = serde_json::to_string(&body).map_err(|err| EthError::Response(err.to_string()))?;
        let response = ureq::post(&self.rpc_url)
            .set("Content-Type", "application/json")
            .send_string(&body_str)
            .map_err(|err| EthError::Http(err.to_string()))?;

        if !(200..300).contains(&response.status()) {
            return Err(EthError::Http(format!("status {}", response.status())));
        }

        let text = response
            .into_string()
            .map_err(|err| EthError::Response(err.to_string()))?;
        let payload: RpcResponse = serde_json::from_str(&text)
            .map_err(|err| EthError::Response(err.to_string()))?;

        if let Some(error) = payload.error {
            return Err(EthError::Rpc(error.message));
        }

        payload
            .result
            .ok_or_else(|| EthError::Response("missing result".into()))
    }
}

fn parse_hex_u64(value: &serde_json::Value) -> Result<u64, EthError> {
    let raw = value
        .as_str()
        .ok_or_else(|| EthError::Response("expected hex string".into()))?;
    let trimmed = raw.trim_start_matches("0x");
    u64::from_str_radix(trimmed, 16).map_err(|err| EthError::Response(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_values() {
        assert_eq!(
            parse_hex_u64(&serde_json::json!("0x7a")).unwrap(),
            122
        );
    }
}
