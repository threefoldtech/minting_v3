use axum::{
    body::{boxed, Full},
    extract::Path,
    http::{header, Method, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use receipt::{FixupReceipt, GenericReceipt, KeyedReceipt, MintingReceipt, RetryPayoutReceipt};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    sync::Arc,
};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
};

mod period;
mod receipt;

#[derive(RustEmbed)]
#[folder = "minting_ui/build/"]
struct Asset;

struct ReceiptStore {
    receipts: HashMap<String, receipt::GenericReceipt>,
    /// receipts by node id
    id_receipts: BTreeMap<u32, Vec<String>>,
}

impl ReceiptStore {
    fn new() -> Self {
        Self {
            receipts: HashMap::new(),
            id_receipts: BTreeMap::new(),
        }
    }

    fn add_minting_receipt(&mut self, hash: String, receipt: MintingReceipt) {
        self.id_receipts
            .entry(receipt.node_id)
            .or_default()
            .push(hash.clone());
        self.receipts
            .insert(hash, GenericReceipt::Minting(receipt.clone()));
    }

    fn add_retry_receipt(&mut self, hash: String, receipt: RetryPayoutReceipt) {
        self.receipts.insert(hash, GenericReceipt::Retry(receipt));
    }

    fn add_fixup_receipt(&mut self, hash: String, receipt: FixupReceipt) {
        self.id_receipts
            .entry(receipt.node_id)
            .or_default()
            .push(hash.clone());
        self.receipts.insert(hash, GenericReceipt::Fixup(receipt));
    }
}

#[tokio::main]
async fn main() {
    let mut store = ReceiptStore::new();
    let mut base_dir = std::path::PathBuf::new();
    base_dir.push("receipts");

    for dir in std::fs::read_dir(&base_dir).unwrap() {
        let dir = dir.unwrap();
        if !dir.file_type().unwrap().is_dir() {
            continue;
        }
        let mut path = base_dir.clone();
        path.push(dir.file_name());
        for file in std::fs::read_dir(&path).unwrap() {
            let file = file.unwrap();
            if !file.file_type().unwrap().is_file() {
                continue;
            }
            let mut path = path.clone();
            path.push(file.file_name());
            let data = std::fs::read_to_string(path).unwrap();
            let receipt: MintingReceipt = serde_json::from_str(&data).unwrap();
            store.add_minting_receipt(file.file_name().into_string().unwrap(), receipt);
        }
    }

    let mut fixup_dir = base_dir.clone();
    fixup_dir.push("fixed");
    for dir in std::fs::read_dir(&fixup_dir).unwrap() {
        let dir = dir.unwrap();
        if !dir.file_type().unwrap().is_dir() {
            continue;
        }
        let mut path = fixup_dir.clone();
        path.push(dir.file_name());
        for file in std::fs::read_dir(&path).unwrap() {
            let file = file.unwrap();
            if !file.file_type().unwrap().is_file() {
                continue;
            }
            let mut path = path.clone();
            path.push(file.file_name());
            let data = std::fs::read_to_string(path).unwrap();
            let receipt: FixupReceipt = serde_json::from_str(&data).unwrap();
            store.add_fixup_receipt(file.file_name().into_string().unwrap(), receipt);
        }
    }

    let mut retry_dir = base_dir.clone();
    retry_dir.push("retries");
    for dir in std::fs::read_dir(&retry_dir).unwrap() {
        let dir = dir.unwrap();
        if !dir.file_type().unwrap().is_dir() {
            continue;
        }
        let mut path = retry_dir.clone();
        path.push(dir.file_name());
        for file in std::fs::read_dir(&path).unwrap() {
            let file = file.unwrap();
            if !file.file_type().unwrap().is_file() {
                continue;
            }
            let mut path = path.clone();
            path.push(file.file_name());
            let data = std::fs::read_to_string(path).unwrap();
            let receipt: RetryPayoutReceipt = serde_json::from_str(&data).unwrap();
            store.add_retry_receipt(file.file_name().into_string().unwrap(), receipt);
        }
    }

    let rs = Arc::new(store);
    let app = Router::new()
        .route("/", get(index))
        .fallback(get(static_handler))
        .route("/api/v1/receipt/:receipt_hash", get(receipt_by_hash))
        .route("/api/v1/node/:node_id", get(receipts_for_node))
        .layer(CompressionLayer::new())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET])
                .allow_headers(Any),
        )
        .layer(Extension(rs));

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn receipt_by_hash(
    Extension(receipt_store): Extension<Arc<ReceiptStore>>,
    Path(receipt_hash): Path<String>,
) -> Result<Json<receipt::GenericReceipt>, StatusCode> {
    if let Some(receipt) = receipt_store.receipts.get(&receipt_hash).map(Clone::clone) {
        Ok(Json(receipt))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn receipts_for_node(
    Extension(receipt_store): Extension<Arc<ReceiptStore>>,
    Path(node_id): Path<u32>,
) -> Result<Json<Vec<KeyedReceipt>>, StatusCode> {
    if let Some(hashes) = receipt_store.id_receipts.get(&node_id).map(Clone::clone) {
        Ok(Json(
            hashes
                .into_iter()
                .map(|hash| {
                    let receipt = receipt_store
                        .receipts
                        .get(&hash)
                        .expect("indexed receipt is present in receipt list")
                        .clone();
                    KeyedReceipt { hash, receipt }
                })
                .collect(),
        ))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn index() -> impl IntoResponse {
    static_handler("/index.html".parse::<Uri>().unwrap()).await
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/').to_string();

    StaticFile(path)
}

pub struct StaticFile<T>(pub T);

impl<T> IntoResponse for StaticFile<T>
where
    T: Into<String>,
{
    fn into_response(self) -> Response {
        let path = self.0.into();

        match Asset::get(path.as_str()) {
            Some(content) => {
                let body = boxed(Full::from(content.data));
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                Response::builder()
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(body)
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(boxed(Full::from("404")))
                .unwrap(),
        }
    }
}
