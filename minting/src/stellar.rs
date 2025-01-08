use std::collections::HashMap;
use stellar_rs::horizon_client::HorizonClient;
use stellar_rs::models::Order;
use stellar_rs::transactions::prelude::TransactionsForAccountRequest;

const TFT_ISSUER: &str = "GBOVQKJYHXRR3DX6NOX2RRYFRCUMSADGDESTDNBDS6CDVLGVESRTAC47";
const PAGE_LIMIT: u8 = 100;

pub struct Horizon {
    client: HorizonClient,
}

impl Horizon {
    /// Create a new horizon instance.
    pub fn new(url: &str) -> Self {
        Self {
            client: HorizonClient::new(url).unwrap(),
        }
    }

    pub async fn filter_previous_mints<V, W>(
        &self,
        mint_map: &mut HashMap<[u8; 32], V>,
        other_mint_map: &mut HashMap<[u8; 32], W>,
    ) {
        let mut transactions_request = TransactionsForAccountRequest::new()
            .set_order(Order::Asc)
            .expect("Can set order")
            .set_limit(PAGE_LIMIT)
            .expect("Can set limit")
            .set_account_id(TFT_ISSUER)
            .expect("Can set acccount");
        loop {
            let transactions_response = self
                .client
                .get_transactions_for_account(&transactions_request)
                .await
                .unwrap();

            for tx in transactions_response.embedded().records() {
                println!("{}", tx.hash());
                if tx.memo_type() != "hash" {
                    continue;
                }
                if let Some(ref memo) = tx.memo() {
                    let mut hash = [0; 32];
                    base64::decode_config_slice(memo, base64::STANDARD, &mut hash).unwrap();
                    mint_map.remove(&hash);
                    other_mint_map.remove(&hash);
                }
            }

            if transactions_response.embedded().records().len() < PAGE_LIMIT as usize {
                break;
            }

            transactions_request = transactions_request
                .set_cursor(
                    transactions_response.embedded().records()
                        [transactions_response.embedded().records().len() - 1]
                        .paging_token()
                        .clone(),
                )
                .expect("Can set cursor");
        }
    }
}
