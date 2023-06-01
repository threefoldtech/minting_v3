use chrono::prelude::*;
use serde::{Deserialize, Serialize};
pub use sp_application_crypto::ed25519;
pub use sp_core::crypto::AccountId32;
pub use sp_core::H256 as Hash;
use std::fmt::{self, Display};
// pub use substrate_api_client::{AccountData, AccountInfo};
// pub use support::traits::BalanceStatus;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum NodeCertification {
    Diy,
    Certified,
}

impl Default for NodeCertification {
    fn default() -> Self {
        NodeCertification::Diy
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct FarmingPolicy<BlockNumber> {
    pub version: u32,
    pub id: u32,
    pub name: String,
    pub cu: u32,
    pub su: u32,
    pub nu: u32,
    pub ipv4: u32,
    pub minimal_uptime: u16,
    pub policy_created: BlockNumber,
    pub policy_end: BlockNumber,
    pub immutable: bool,
    pub default: bool,
    pub node_certification: NodeCertification,
    pub farm_certification: FarmCertification,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Policy {
    pub value: u32,
    pub unit: Unit,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum Unit {
    Bytes,
    Kilobytes,
    Megabytes,
    Gigabytes,
    Terrabytes,
}

impl Default for Unit {
    fn default() -> Unit {
        Unit::Gigabytes
    }
}
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Resources {
    pub hru: u64,
    pub sru: u64,
    pub cru: u64,
    pub mru: u64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct ContractResources {
    pub contract_id: u64,
    pub used: Resources,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct PricingPolicy {
    pub version: u32,
    pub id: u32,
    pub name: Vec<u8>,
    pub su: Policy,
    pub cu: Policy,
    pub nu: Policy,
    pub ipu: Policy,
    pub unique_name: Policy,
    pub domain_name: Policy,
    pub foundation_account: AccountId32,
    pub certified_sales_account: AccountId32,
    pub discount_for_dedication_nodes: u8,
}

/// A list of Grandpa authorities with associated weights.
pub type AuthorityList = Vec<(AuthorityId, AuthorityWeight)>;

/// The grandpa authority ID type.
pub type AuthorityId = ed25519::Public;

/// The weight of an authority.
pub type AuthorityWeight = u64;

#[derive(Debug, Serialize, Deserialize)]
pub struct Balance(u64);

impl From<u64> for Balance {
    fn from(amount: u64) -> Self {
        Balance(amount)
    }
}

impl Balance {
    /// Get the balance as the amount of units expressed as a u64.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

pub type BlockNumber = u32;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct Twin {
    pub version: u32,
    pub id: u32,
    pub account_id: AccountId32,
    pub ip: String,
    pub entities: Vec<EntityProof>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub version: u32,
    pub id: u32,
    pub name: Vec<u8>,
    pub account_id: AccountId32,
    pub country: Vec<u8>,
    pub city: Vec<u8>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct EntityProof {
    pub entity_id: u32,
    pub signature: Vec<u8>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Farm {
    pub version: u32,
    pub id: u32,
    pub name: String,
    pub twin_id: u32,
    pub pricing_policy_id: u32,
    pub certification: FarmCertification,
    pub public_ips: Vec<PublicIP>,
    pub dedicated_farm: bool,
    pub farming_policy_limits: Option<FarmingPolicyLimit>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum FarmCertification {
    NotCertified,
    Gold,
}

impl Default for FarmCertification {
    fn default() -> Self {
        FarmCertification::NotCertified
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct FarmingPolicyLimit {
    pub farming_policy_id: u32,
    pub cu: Option<u64>,
    pub su: Option<u64>,
    pub end: Option<u64>,
    pub node_count: Option<u32>,
    pub node_certification: bool,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct PublicIP {
    pub ip: String,
    pub gateway: String,
    pub contract_id: u64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Node {
    pub version: u32,
    pub id: u32,
    pub farm_id: u32,
    pub twin_id: u32,
    pub resources: Resources,
    pub location: Location,
    pub country: String,
    pub city: String,
    // optional public config
    pub public_config: Option<PublicConfig>,
    pub created: u64,
    pub farming_policy_id: u32,
    pub interfaces: Vec<Interface>,
    pub certification: NodeCertification,
    pub secure_boot: bool,
    pub virtualized: bool,
    pub serial_number: String,
    pub connection_price: u32,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Location {
    pub longitude: String,
    pub latitude: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Interface {
    pub name: String,
    pub mac: String,
    pub ips: Vec<IP>,
}

pub type IP = String;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct PublicConfig {
    pub ipv4: String,
    pub ipv6: String,
    pub gw4: String,
    pub gw6: String,
    pub domain: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum DiscountLevel {
    None,
    Default,
    Bronze,
    Silver,
    Gold,
}

impl Default for DiscountLevel {
    fn default() -> DiscountLevel {
        DiscountLevel::None
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Consumption {
    pub contract_id: u64,
    pub timestamp: u64,
    pub cru: u64,
    pub sru: u64,
    pub hru: u64,
    pub mru: u64,
    pub nru: u64,
}
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct ContractBill {
    pub contract_id: u64,
    pub timestamp: u64,
    pub discount_level: DiscountLevel,
    pub amount_billed: u128,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Contract {
    pub version: u32,
    pub state: ContractState,
    pub contract_id: u64,
    pub twin_id: u32,
    pub contract_type: ContractData,
}
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct NodeContract {
    pub node_id: u32,
    // deployment_data is the encrypted deployment body. This encrypted the deployment with the **USER** public key.
    // So only the user can read this data later on (or any other key that he keeps safe).
    // this data part is read only by the user and can actually hold any information to help him reconstruct his deployment or can be left empty.
    pub deployment_data: Vec<u8>,
    // Hash of the deployment, set by the user
    pub deployment_hash: Vec<u8>,
    pub public_ips: u32,
    pub public_ips_list: Vec<PublicIP>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct NameContract {
    pub name: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct RentContract {
    pub node_id: u32,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum ContractData {
    NodeContract(NodeContract),
    NameContract(NameContract),
    RentContract(RentContract),
}

impl Default for ContractData {
    fn default() -> ContractData {
        ContractData::NodeContract(NodeContract::default())
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum ContractState {
    Created,
    Deleted(Cause),
    GracePeriod(u64),
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum Cause {
    CanceledByUser,
    OutOfFunds,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct CertificationCodes {
    pub version: u32,
    pub id: u32,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub certification_code_type: CertificationCodeType,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum CertificationCodeType {
    Farm,
    Entity,
}

impl Default for CertificationCodeType {
    fn default() -> Self {
        CertificationCodeType::Farm
    }
}

/// MintTransaction contains all the information about
/// Stellar -> TF Chain minting transaction.
/// if the votes field is larger then (number of validators / 2) + 1 , the transaction will be minted
#[derive(Debug, Serialize, Deserialize)]
pub struct MintTransaction {
    pub amount: Balance,
    pub target: AccountId32,
    pub block: BlockNumber,
    pub votes: u32,
}

/// BurnTransaction contains all the information about
/// TF Chain -> Stellar burn transaction
/// Transaction is ready when (number of validators / 2) + 1 signatures are present
#[derive(Debug, Serialize, Deserialize)]
pub struct BurnTransaction {
    pub block: BlockNumber,
    pub amount: Balance,
    pub target: Vec<u8>,
    pub signatures: Vec<StellarSignature>,
    pub sequence_number: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RefundTransaction {
    pub block: BlockNumber,
    pub amount: Balance,
    pub target: Vec<u8>,
    pub tx_hash: Vec<u8>,
    pub signatures: Vec<StellarSignature>,
    pub sequence_number: u64,
}
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Default, Debug, Serialize, Deserialize)]
pub struct StellarSignature {
    pub signature: Vec<u8>,
    pub stellar_pub_key: Vec<u8>,
}

impl Default for ContractState {
    fn default() -> ContractState {
        ContractState::Created
    }
}

impl Display for Twin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Twin details for twin {}", self.id)?;
        writeln!(f, "Account ID {}", self.account_id)?;
        writeln!(f, "IP: {}", self.ip)
    }
}

impl Display for ContractData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractData::NameContract(name_contract) => {
                writeln!(f, "Name: {}", name_contract.name)
            }
            ContractData::NodeContract(node_contract) => {
                writeln!(f, "Node id: {}", node_contract.node_id)?;
                writeln!(
                    f,
                    "Deployment data: {}",
                    String::from_utf8_lossy(&node_contract.deployment_data)
                )?;
                writeln!(
                    f,
                    "Deployment hash: {}",
                    String::from_utf8_lossy(&node_contract.deployment_hash)
                )?;
                for ip in &node_contract.public_ips_list {
                    writeln!(f, "IP: {}", ip.ip)?;
                    writeln!(f, "Gateway: {}", ip.gateway)?;
                }
                write!(f, "Number of public ips: {}", node_contract.public_ips)
            }
            ContractData::RentContract(rent_contract) => {
                writeln!(f, "Rented node id: {}", rent_contract.node_id)
            }
        }
    }
}

impl Display for Contract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Contract details for contract {}", self.contract_id)?;
        writeln!(f, "State: {}", self.state)?;
        writeln!(f, "Twin id: {}", self.twin_id)?;
        writeln!(f, "{}", self.contract_type)
    }
}

impl Display for ContractState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractState::Created => {
                write!(f, "Created")
            }
            ContractState::Deleted(Cause::CanceledByUser) => {
                write!(f, "Canceled by user")
            }
            ContractState::Deleted(Cause::OutOfFunds) => {
                write!(f, "Out of funds")
            }
            ContractState::GracePeriod(bn) => {
                write!(f, "In grace period until block {}", bn)
            }
        }
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Node details for node {}", self.id)?;
        writeln!(f, "Farm ID: {}", self.farm_id)?;
        writeln!(f, "Twin ID: {}", self.twin_id)?;
        writeln!(f, "Node resources:")?;
        writeln!(f, "\tCRU: {} (logical cores)", self.resources.cru)?;
        writeln!(
            f,
            "\tMRU: {} ({:.02} GB | {:.02} GiB)",
            self.resources.mru,
            self.resources.mru as f64 / 1_000_000_000f64,
            self.resources.mru as f64 / (1 << 30) as f64
        )?;
        writeln!(
            f,
            "\tSRU: {} ({:.02} TB | {:.02} TiB)",
            self.resources.sru,
            self.resources.sru as f64 / 1_000_000_000_000f64,
            self.resources.sru as f64 / (1u64 << 40) as f64
        )?;
        writeln!(
            f,
            "\tHRU: {} ({:.02} TB | {:.02} TiB)",
            self.resources.hru,
            self.resources.hru as f64 / 1_000_000_000_000f64,
            self.resources.hru as f64 / (1u64 << 40) as f64
        )?;
        writeln!(
            f,
            "Location: {}, {} ({} lat {} long)",
            self.city, self.country, self.location.latitude, self.location.longitude
        )?;
        writeln!(
            f,
            "Created at: {}",
            Utc.timestamp(self.created as i64, 0)
                .with_timezone(&Local)
                .to_rfc2822()
        )?;
        writeln!(f, "Farming policy: {}", self.farming_policy_id)?;
        if self.interfaces.is_empty() {
            writeln!(f, "No known interfaces")?;
        } else {
            // Add some nice formatting, make sure all MAC address and IP addresses start in the
            // same column.
            // unwrap here is safe as None is only returned in case of an empty iterator, and we
            // specifically check for that case.
            let iface_name_space = self
                .interfaces
                .iter()
                .map(|iface| iface.name.len())
                .max()
                .unwrap();
            writeln!(f, "Interfaces:")?;
            for iface in &self.interfaces {
                writeln!(
                    f,
                    "\t{}: {:>width$}",
                    iface.name,
                    iface.mac,
                    width = iface_name_space - iface.name.len() + iface.mac.len()
                )?;
                for ip in &iface.ips {
                    writeln!(
                        f,
                        "\t{:>width$}",
                        ip,
                        width = ip.len() + iface_name_space + 2
                    )?;
                }
            }
        }
        if let Some(ref pub_config) = self.public_config {
            f.pad("Public config:\n")?;
            f.pad(&format!(
                "\tIPv4: {} (gw: {})\n",
                pub_config.ipv4, pub_config.gw4
            ))?;
            f.pad(&format!(
                "\tIPv6: {} (gw: {})\n",
                pub_config.ipv6, pub_config.gw6
            ))?;
            f.pad(&format!("\tDomain: {}\n", pub_config.domain))?;
        }
        f.pad(&format!(
            "Certification type {}\n",
            match self.certification {
                NodeCertification::Diy => "DIY",
                NodeCertification::Certified => "Certified",
            }
        ))?;
        f.pad(&format!("Secure boot enabled: {}\n", self.secure_boot))?;
        f.pad(&format!("Virtualized: {}\n", self.virtualized))?;
        f.pad(&format!("MOBO serial number: {}\n", self.serial_number))
    }
}

impl Display for Farm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Farm details for farm {} (ID: {})", self.name, self.id)?;
        writeln!(f, "Twin ID: {}", self.twin_id)?;
        writeln!(
            f,
            "Certification type {}",
            match self.certification {
                FarmCertification::NotCertified => "Not Certified",
                FarmCertification::Gold => "Gold",
            }
        )?;
        if !self.public_ips.is_empty() {
            writeln!(f, "Public IPs:")?;
            for ip in &self.public_ips {
                writeln!(f, "\tIPv4: {} (gw: {})", ip.ip, ip.gateway)?;
                writeln!(f, "\tContract id: {}", ip.contract_id)?;
            }
        }
        writeln!(f, "version: {}", self.version)
    }
}
