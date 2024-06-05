use std::collections::HashMap;
use stellar_base::PublicKey;
use stellar_horizon::{
    api,
    client::{HorizonClient, HorizonHttpClient},
    request::{Order, PageRequest},
};

const TFT_ISSUER: &str = "GBOVQKJYHXRR3DX6NOX2RRYFRCUMSADGDESTDNBDS6CDVLGVESRTAC47";
const PAGE_LIMIT: u64 = 100;

pub struct Horizon {
    client: HorizonHttpClient,
}

impl Horizon {
    /// Create a new horizon instance.
    pub fn new(url: &str) -> Self {
        Self {
            client: HorizonHttpClient::new_from_str(url).unwrap(),
        }
    }

    pub async fn filter_previous_mints<V, W>(
        &self,
        mint_map: &mut HashMap<[u8; 32], V>,
        other_mint_map: &mut HashMap<[u8; 32], W>,
    ) {
        let mut cursor = "".to_string();
        loop {
            let req =
                api::transactions::for_account(&PublicKey::from_account_id(TFT_ISSUER).unwrap())
                    .with_cursor(&cursor)
                    .with_limit(PAGE_LIMIT)
                    .with_order(&Order::Ascending);
            let (_, resp) = self.client.request(req).await.unwrap();
            for tx in &resp.records {
                // TODO:
                //   - b64 decode memo
                //   - remove entry from mint map
                if tx.memo_type != "hash" {
                    continue;
                }
                // infallible
                if let Some(ref memo) = tx.memo {
                    let mut hash = [0; 32];
                    base64::decode_config_slice(memo, base64::STANDARD, &mut hash).unwrap();
                    mint_map.remove(&hash);
                    other_mint_map.remove(&hash);
                }
            }
            if resp.records.len() < PAGE_LIMIT as usize {
                break;
            }
            // TODO: rework so this clone can go
            cursor.clone_from(&resp.records[resp.records.len() - 1].paging_token);
        }
    }
}
