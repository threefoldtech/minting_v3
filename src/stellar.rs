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

    pub fn filter_previous_mints<V>(&self, mint_map: &mut HashMap<[u8; 32], V>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut cursor = "".to_string();
            loop {
                let req = api::transactions::for_account(
                    &PublicKey::from_account_id(TFT_ISSUER).unwrap(),
                )
                .with_cursor(&cursor)
                .with_limit(PAGE_LIMIT)
                .with_order(&Order::Ascending);
                let (_, resp) = self.client.request(req).await.unwrap();
                for tx in &resp.records {
                    // TODO:
                    //   - memo hash
                    //   - b64 decode memo
                    //   - remove entry from mint map
                    if tx.memo_type != "hash" {
                        continue;
                    }
                    println!("{:#?}", tx);
                }
                if resp.records.len() < PAGE_LIMIT as usize {
                    break;
                }
                // TODO: rework so this clone can go
                cursor = resp.records[resp.records.len() - 1].paging_token.clone();
            }
        });
    }
}
