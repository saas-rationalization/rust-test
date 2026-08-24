use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Serialize;
use tiny_http::{Header, Method, Response, Server, StatusCode};
use thiserror::Error;

use crate::chain::ChainStore;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("chain error: {0}")]
    Chain(#[from] crate::chain::ChainError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn serve<P: AsRef<Path>>(data_dir: P, port: u16) -> Result<(), NodeError> {
    let data_dir = data_dir.as_ref().to_path_buf();
    let store = Arc::new(Mutex::new(ChainStore::open_or_init(&data_dir)?));
    let server = Server::http(format!("127.0.0.1:{port}"))
        .map_err(|err| NodeError::Http(err.to_string()))?;

    let handle = thread::spawn(move || {
        for request in server.incoming_requests() {
            let response = handle_request(&store, request.method(), request.url());
            let _ = request.respond(response);
        }
    });

    handle
        .join()
        .map_err(|_| NodeError::Http("node thread panicked".into()))?;
    Ok(())
}

fn handle_request(store: &Arc<Mutex<ChainStore>>, method: &Method, url: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    match (method, url) {
        (&Method::Get, "/v1/status") => json_response(&status_payload(store))
            .unwrap_or_else(|err| error_response(StatusCode(500), &err.to_string())),
        (&Method::Get, url) if url.starts_with("/v1/balance?address=") => {
            match query_param(url, "address") {
                Some(address) => match store.lock().unwrap().balance(address) {
                    Ok(balance) => json_response(&serde_json::json!({
                        "address": address,
                        "balance": balance
                    }))
                    .unwrap_or_else(|err| error_response(StatusCode(500), &err.to_string())),
                    Err(err) => error_response(StatusCode(400), &err.to_string()),
                },
                None => error_response(StatusCode(400), "missing address"),
            }
        }
        (&Method::Get, url) if url.starts_with("/v1/history?address=") => {
            match query_param(url, "address") {
                Some(address) => match store.lock().unwrap().history(address) {
                    Ok(entries) => {
                        let payload: Vec<_> = entries
                            .into_iter()
                            .map(|entry| {
                                serde_json::json!({
                                    "block_height": entry.block_height,
                                    "tx_id": entry.tx_id,
                                    "direction": format!("{:?}", entry.direction),
                                    "counterparty": entry.counterparty,
                                    "amount": entry.amount,
                                    "fee": entry.fee,
                                })
                            })
                            .collect();
                        json_response(&payload)
                            .unwrap_or_else(|err| error_response(StatusCode(500), &err.to_string()))
                    }
                    Err(err) => error_response(StatusCode(400), &err.to_string()),
                },
                None => error_response(StatusCode(400), "missing address"),
            }
        }
        _ => error_response(StatusCode(404), "not found"),
    }
}

fn status_payload(store: &Arc<Mutex<ChainStore>>) -> serde_json::Value {
    let chain = store.lock().unwrap();
    serde_json::json!({
        "chain_id": crate::chain::CHAIN_ID,
        "height": chain.height(),
        "tip": chain.tip_hash(),
        "pending": chain.pending_count(),
        "difficulty": chain.difficulty(),
        "data_dir": chain.data_dir().display().to_string(),
    })
}

fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split('?')
        .nth(1)?
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
}

fn json_response<T: Serialize>(payload: &T) -> Result<Response<std::io::Cursor<Vec<u8>>>, NodeError> {
    let body = serde_json::to_vec_pretty(payload)?;
    Ok(Response::from_data(body).with_header(
        Header::from_bytes("Content-Type", "application/json").expect("valid header"),
    ))
}

fn error_response(status: StatusCode, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    #[derive(Serialize)]
    struct ErrorBody<'a> {
        error: &'a str,
    }

    let body = serde_json::to_vec(&ErrorBody { error: message }).unwrap_or_default();
    Response::from_data(body)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}
