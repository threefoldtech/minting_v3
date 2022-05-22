//! Copy of the `receipt` module from the `minting_v3` project. The module is copied since we don't
//! want to build (multiple) tfchain runtimes for this project. Additionally, an enum is added
//! containing all possible types of receipts.

use crate::period::Period;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
/// A combination of all known v3 receipts.
pub enum GenericReceipt {
    Minting(MintingReceipt),
    Retry(RetryPayoutReceipt),
    Fixup(FixupReceipt),
}

#[derive(Serialize, Deserialize, Clone)]
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

#[derive(Clone, Copy, Serialize, Deserialize)]
/// Cloud units for a node.
pub struct CloudUnits {
    pub cu: f64,
    pub su: f64,
    pub nu: f64,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
/// Payout for a node.
pub struct Reward {
    /// Reward in milli USD.
    pub musd: u64,
    /// Reward in TFT units. 1 TFT -> 1e7 units.
    pub tft: u64,
}

#[derive(Serialize, Deserialize, Clone)]
/// Resource units as reported by the node.
pub struct ResourceUnits {
    pub cru: f64,
    pub mru: f64,
    pub hru: f64,
    pub sru: f64,
}

#[derive(Serialize, Deserialize, Clone)]
/// Utilization of resoures on a node as measured through capacity reports on the chain.
pub struct ResourceUtilization {
    pub cru: f64,
    pub mru: f64,
    pub hru: f64,
    pub sru: f64,
    pub ip: f64,
}

#[derive(Serialize, Deserialize, Clone)]
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

#[derive(Serialize, Deserialize, Clone)]
/// A receipt to correct underpayment of nodes in february 2022.
pub struct FixupReceipt {
    pub period: Period,
    pub node_id: u32,
    pub farm_id: u32,
    pub minted_cloud_units: CloudUnits,
    pub correct_cloud_units: CloudUnits,
    pub fixup_cloud_units: CloudUnits,
    pub stellar_payout_address: String,
    pub minted_receipt: String,
    pub correct_receipt: String,
    pub minted_reward: Reward,
    pub correct_reward: Reward,
    pub fixup_reward: Reward,
    pub minted_carbon_offset: Reward,
    pub correct_carbon_offset: Reward,
    pub fixup_carbon_offset: Reward,
}

#[derive(Serialize, Deserialize, Clone)]
/// A receipt combined with its hash
pub struct KeyedReceipt {
    pub hash: String,
    pub receipt: GenericReceipt,
}
