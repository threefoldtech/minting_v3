use crate::period::Period;
use blake2::{digest::consts::U32, Blake2b, Digest};
use serde::{Deserialize, Serialize};

type Blake2b256 = Blake2b<U32>;

#[derive(Serialize, Deserialize)]
/// A receipt which will be stored to validate the payout of a node. This will then be hashed to
/// create the payment memo.
///
/// Note that this only makes sense for valid mints, hence there is no error field here.
pub struct MintingReceipt {
    pub period: Period,
    pub node_id: u32,
    pub twin_id: u32,
    pub farm_id: u32,
    pub farm_name: String,
    pub stellar_payout_address: String,
    pub measured_uptime: u64,
    /// TFT price on connection in milli USD.
    pub tft_connection_price: u64,
    pub cloud_units: CloudUnits,
    pub resource_units: ResourceUnits,
    pub resource_utilization: ResourceUtilization,
    pub reward: Reward,
    pub carbon_offset: Reward,
    /// Certification type of the node, "Certified" or "DIY".
    pub node_type: String,
}

impl MintingReceipt {
    /// Get the hash of the receipt.
    pub fn hash(&self) -> [u8; 32] {
        let out = serde_json::to_vec(&self).unwrap();
        let mut hasher = Blake2b256::new();
        hasher.update(out);
        hasher.finalize().into()
    }
}

#[derive(Serialize, Deserialize)]
/// Cloud units for a node.
pub struct CloudUnits {
    pub cu: f64,
    pub su: f64,
    pub nu: f64,
}

#[derive(Serialize, Deserialize)]
/// Payout for a node.
pub struct Reward {
    /// Reward in milli USD.
    pub musd: u64,
    /// Reward in TFT units. 1 TFT -> 1e7 units.
    pub tft: u64,
}

#[derive(Serialize, Deserialize)]
/// Resource units as reported by the node.
pub struct ResourceUnits {
    pub cru: f64,
    pub mru: f64,
    pub hru: f64,
    pub sru: f64,
}

#[derive(Serialize, Deserialize)]
/// Utilization of resoures on a node as measured through capacity reports on the chain.
pub struct ResourceUtilization {
    pub cru: f64,
    pub mru: f64,
    pub hru: f64,
    pub sru: f64,
    pub ip: f64,
}

#[derive(Serialize, Deserialize)]
/// A receipt which will be stored to validate the payout of a node. This will then be hashed to
/// create the payment memo.
///
/// Note that this only makes sense for valid mints, hence there is no error field here.
pub struct RetryPayoutReceipt {
    pub failed_payout_period: Period,
    pub retry_period: Period,
    pub farm_id: u32,
    pub previous_stellar_payout_address: String,
    pub stellar_payout_address: String,
    pub retry_for_receipt: String,
    pub reward: Reward,
}

impl RetryPayoutReceipt {
    /// Get the hash of the receipt.
    pub fn hash(&self) -> [u8; 32] {
        let out = serde_json::to_vec(&self).unwrap();
        let mut hasher = Blake2b256::new();
        hasher.update(out);
        hasher.finalize().into()
    }
}

#[derive(Serialize, Deserialize)]
/// A receipt to correct underpayment of nodes in february 2022.
pub struct FixupReceipt {
    pub perod: Period,
    pub node_id: u32,
    pub farm_id: u32,
    pub stellar_payout_address: String,
    pub incorrect_receipt: String,
    pub received_payout: Reward,
    pub correct_payout: Reward,
    pub fixup_payout: Reward,
    pub previous_calculated_cu: f64,
    pub actual_cu: f64,
}

impl FixupReceipt {
    /// Get the hash of the receipt.
    pub fn hash(&self) -> [u8; 32] {
        let out = serde_json::to_vec(&self).unwrap();
        let mut hasher = Blake2b256::new();
        hasher.update(out);
        hasher.finalize().into()
    }
}
