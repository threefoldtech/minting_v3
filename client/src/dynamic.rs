use crate::client::RuntimeClient;
use crate::runtimes::{
    v115::types::{
        V115Contract, V115ContractCreatedEvent, V115ContractNruConsumptionReceivedEvent,
        V115ContractResources, V115ContractUpdatedResourcesEvent, V115Farm, V115FarmingPolicy,
        V115Node, V115NodeStoredEvent, V115NodeUpdatedEvent, V115NodeUptimeReportedEvent, V115Twin,
    },
    v123::types::{
        V123Contract, V123ContractCreatedEvent, V123ContractNruConsumptionReceivedEvent,
        V123ContractResources, V123ContractUpdatedResourcesEvent, V123Farm, V123FarmingPolicy,
        V123Node, V123NodeStoredEvent, V123NodeUpdatedEvent, V123NodeUptimeReportedEvent, V123Twin,
    },
    v131::types::{
        V131Contract, V131ContractCreatedEvent, V131ContractNruConsumptionReceivedEvent,
        V131ContractResources, V131ContractUpdatedResourcesEvent, V131Farm, V131FarmingPolicy,
        V131Node, V131NodePower, V131NodeStoredEvent, V131NodeUpdatedEvent,
        V131NodeUptimeReportedEvent, V131PowerStateChangedEvent, V131PowerTargetChangedEvent,
        V131Twin,
    },
    v141::types::{
        V141Contract, V141ContractCreatedEvent, V141ContractNruConsumptionReceivedEvent,
        V141ContractResources, V141ContractUpdatedResourcesEvent, V141Farm, V141FarmingPolicy,
        V141Node, V141NodePower, V141NodeStoredEvent, V141NodeUpdatedEvent,
        V141NodeUptimeReportedEvent, V141PowerStateChangedEvent, V141PowerTargetChangedEvent,
        V141Twin,
    },
};
use crate::types::{
    Contract, ContractResources, Farm, FarmPolicy, Hash, Node, NodePower, RuntimeEvents, Twin,
    CONTRACTS, CONTRACT_CREATED, CONTRACT_ID, FARMING_POLICIES, FARMING_POLICY_ID, FARMS, FARM_ID,
    FARM_PAYOUT_V2_ADDRESS, NODES, NODE_CONTRACT_RESOURCES, NODE_ID, NODE_POWER, NODE_STORED,
    NODE_UPDATED, NODE_UPTIME_REPORTED, NRU_CONSUMPTION_RECEIVED, POWER_STATE_CHANGED,
    POWER_TARGET_CHANGED, SMART_CONTRACT_MODULE, TFGRID_MODULE, TIMESTAMP_MODULE, TIMESTAMP_NOW,
    TWINS, TWIN_ID, UPDATE_USED_RESOURCES,
};
use std::{error, fmt};
use subxt::storage::DynamicStorageAddress;
use subxt::{
    dynamic::Value,
    // events::Events,
    rpc::types::{BlockNumber, NumberOrHex},
    OnlineClient,
    PolkadotConfig,
};
use tokio::join;

#[derive(Debug, Clone)]
pub enum Error {
    ErrorDecodingTwin,
    ErrorDecodingFarm,
    ErrorDecodingNode,
    ErrorDecodingContract,
    ErrorDecodingContractResources,
    ErrorDecodingFarmingPolicy,
    ErrorDecodingNodePower,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::ErrorDecodingTwin => write!(f, "failed to decode twin"),
            Error::ErrorDecodingNode => write!(f, "failed to decode node"),
            Error::ErrorDecodingFarm => write!(f, "failed to decode farm"),
            Error::ErrorDecodingContract => write!(f, "failed to decode contract"),
            Error::ErrorDecodingContractResources => {
                write!(f, "failed to decode contract resources")
            }
            Error::ErrorDecodingFarmingPolicy => write!(f, "failed to decode farming policy"),
            Error::ErrorDecodingNodePower => write!(f, "failed to decode node power"),
        }
    }
}

impl error::Error for Error {}

pub struct DynamicClient {
    api: OnlineClient<PolkadotConfig>,
}

impl DynamicClient {
    pub async fn new(url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let api = OnlineClient::from_url(url).await?;
        Ok(DynamicClient { api })
    }
}

#[async_trait::async_trait]
impl RuntimeClient for DynamicClient {
    /// Get all events in a block.
    async fn events(
        &self,
        block: Option<Hash>,
    ) -> Result<Vec<RuntimeEvents>, Box<dyn std::error::Error>> {
        let (meta, runtime_v) = join!(
            self.api.rpc().metadata(block),
            self.api.rpc().runtime_version(block),
        );

        self.api.set_runtime_version(runtime_v?);
        self.api.set_metadata(meta?);

        let b_events = self.api.events().at(block).await?;

        let mut events: Vec<RuntimeEvents> = vec![];
        for event in b_events.iter() {
            if !event.is_ok() {
                continue;
            }
            let evt = event?;

            match (evt.pallet_name(), evt.variant_name()) {
                (TFGRID_MODULE, NODE_STORED) => {
                    if let Ok(Some(evt)) = evt.as_event::<V115NodeStoredEvent>() {
                        events.push(RuntimeEvents::NodeStoredEvent(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V123NodeStoredEvent>() {
                        events.push(RuntimeEvents::NodeStoredEvent(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V131NodeStoredEvent>() {
                        events.push(RuntimeEvents::NodeStoredEvent(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V141NodeStoredEvent>() {
                        events.push(RuntimeEvents::NodeStoredEvent(evt.0.into()));
                    };
                }
                (TFGRID_MODULE, NODE_UPDATED) => {
                    if let Ok(Some(evt)) = evt.as_event::<V115NodeUpdatedEvent>() {
                        events.push(RuntimeEvents::NodeUpdatedEvent(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V123NodeUpdatedEvent>() {
                        events.push(RuntimeEvents::NodeUpdatedEvent(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V131NodeUpdatedEvent>() {
                        events.push(RuntimeEvents::NodeUpdatedEvent(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V141NodeUpdatedEvent>() {
                        events.push(RuntimeEvents::NodeUpdatedEvent(evt.0.into()));
                    };
                }
                (TFGRID_MODULE, NODE_UPTIME_REPORTED) => {
                    if let Ok(Some(evt)) = evt.as_event::<V115NodeUptimeReportedEvent>() {
                        events.push(RuntimeEvents::NodeUptimeReported(evt.0, evt.1, evt.2));
                    } else if let Ok(Some(evt)) = evt.as_event::<V123NodeUptimeReportedEvent>() {
                        events.push(RuntimeEvents::NodeUptimeReported(evt.0, evt.1, evt.2));
                    } else if let Ok(Some(evt)) = evt.as_event::<V131NodeUptimeReportedEvent>() {
                        events.push(RuntimeEvents::NodeUptimeReported(evt.0, evt.1, evt.2));
                    } else if let Ok(Some(evt)) = evt.as_event::<V141NodeUptimeReportedEvent>() {
                        events.push(RuntimeEvents::NodeUptimeReported(evt.0, evt.1, evt.2));
                    };
                }
                (SMART_CONTRACT_MODULE, UPDATE_USED_RESOURCES) => {
                    if let Ok(Some(evt)) = evt.as_event::<V115ContractUpdatedResourcesEvent>() {
                        events.push(RuntimeEvents::ContractUsedResourcesUpdated(evt.0.into()));
                    } else if let Ok(Some(evt)) =
                        evt.as_event::<V123ContractUpdatedResourcesEvent>()
                    {
                        events.push(RuntimeEvents::ContractUsedResourcesUpdated(evt.0.into()));
                    } else if let Ok(Some(evt)) =
                        evt.as_event::<V131ContractUpdatedResourcesEvent>()
                    {
                        events.push(RuntimeEvents::ContractUsedResourcesUpdated(evt.0.into()));
                    } else if let Ok(Some(evt)) =
                        evt.as_event::<V141ContractUpdatedResourcesEvent>()
                    {
                        events.push(RuntimeEvents::ContractUsedResourcesUpdated(evt.0.into()));
                    };
                }
                (SMART_CONTRACT_MODULE, NRU_CONSUMPTION_RECEIVED) => {
                    if let Ok(Some(evt)) = evt.as_event::<V115ContractNruConsumptionReceivedEvent>()
                    {
                        events.push(RuntimeEvents::NruConsumptionReceived(evt.0.into()));
                    } else if let Ok(Some(evt)) =
                        evt.as_event::<V123ContractNruConsumptionReceivedEvent>()
                    {
                        events.push(RuntimeEvents::NruConsumptionReceived(evt.0.into()));
                    } else if let Ok(Some(evt)) =
                        evt.as_event::<V131ContractNruConsumptionReceivedEvent>()
                    {
                        events.push(RuntimeEvents::NruConsumptionReceived(evt.0.into()));
                    } else if let Ok(Some(evt)) =
                        evt.as_event::<V141ContractNruConsumptionReceivedEvent>()
                    {
                        events.push(RuntimeEvents::NruConsumptionReceived(evt.0.into()));
                    };
                }
                (SMART_CONTRACT_MODULE, CONTRACT_CREATED) => {
                    if let Ok(Some(evt)) = evt.as_event::<V115ContractCreatedEvent>() {
                        events.push(RuntimeEvents::ContractCreated(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V123ContractCreatedEvent>() {
                        events.push(RuntimeEvents::ContractCreated(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V131ContractCreatedEvent>() {
                        events.push(RuntimeEvents::ContractCreated(evt.0.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V141ContractCreatedEvent>() {
                        events.push(RuntimeEvents::ContractCreated(evt.0.into()));
                    };
                }
                (TFGRID_MODULE, POWER_STATE_CHANGED) => {
                    if let Ok(Some(evt)) = evt.as_event::<V131PowerStateChangedEvent>() {
                        events.push(RuntimeEvents::PowerStateChanged(evt.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V141PowerStateChangedEvent>() {
                        events.push(RuntimeEvents::PowerStateChanged(evt.into()));
                    };
                }
                (TFGRID_MODULE, POWER_TARGET_CHANGED) => {
                    if let Ok(Some(evt)) = evt.as_event::<V131PowerTargetChangedEvent>() {
                        events.push(RuntimeEvents::PowerTargetChanged(evt.into()));
                    } else if let Ok(Some(evt)) = evt.as_event::<V141PowerTargetChangedEvent>() {
                        events.push(RuntimeEvents::PowerTargetChanged(evt.into()));
                    };
                }
                (_m, _e) => (),
            }
        }
        Ok(events)
    }

    /// Get the hash of a block at the given height. Note that in this case, block is actually the
    /// height rather than the hash to query at.
    async fn hash_at_height(
        &self,
        block: Option<u32>,
    ) -> Result<Option<Hash>, Box<dyn std::error::Error>> {
        Ok(self
            .api
            .rpc()
            .block_hash(block.map(|block| BlockNumber::from(NumberOrHex::from(block))))
            .await?)
    }

    /// Get the on chain timestamp of the block, in seconds since the UNIX epoch.
    async fn timestamp(&self, block: Option<Hash>) -> Result<u64, Box<dyn std::error::Error>> {
        let storage_address: DynamicStorageAddress<Value> =
            subxt::dynamic::storage(TIMESTAMP_MODULE, TIMESTAMP_NOW, vec![]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch_or_default(&storage_address)
            .await?
            .to_value()?;

        Ok(result.as_u128().map_or(0, |x| x as u64))
    }

    /// Get the twin referenced by this ID.
    async fn twin(
        &self,
        id: u32,
        block: Option<Hash>,
    ) -> Result<Option<Twin>, Box<dyn std::error::Error>> {
        let storage_address =
            subxt::dynamic::storage(TFGRID_MODULE, TWINS, vec![Value::u128(id.into())]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        if let Ok(twin) = codec::decode_from_bytes::<V115Twin>(r.clone().into()) {
            Ok(Some(twin.into()))
        } else if let Ok(twin) = codec::decode_from_bytes::<V123Twin>(r.clone().into()) {
            Ok(Some(twin.into()))
        } else if let Ok(twin) = codec::decode_from_bytes::<V131Twin>(r.clone().into()) {
            Ok(Some(twin.into()))
        } else if let Ok(twin) = codec::decode_from_bytes::<V141Twin>(r.clone().into()) {
            Ok(Some(twin.into()))
        } else {
            return Err(Error::ErrorDecodingTwin.into());
        }
    }

    /// Get the amount of twins on the grid.
    async fn twin_count(&self, block: Option<Hash>) -> Result<u32, Box<dyn std::error::Error>> {
        let storage_address: DynamicStorageAddress<Value> =
            subxt::dynamic::storage(TFGRID_MODULE, TWIN_ID, vec![]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch_or_default(&storage_address)
            .await?
            .to_value()?;

        Ok(result.as_u128().map_or(0, |x| x as u32))
    }

    /// Get the farm referenced by this ID.
    async fn farm(
        &self,
        id: u32,
        block: Option<Hash>,
    ) -> Result<Option<Farm>, Box<dyn std::error::Error>> {
        let storage_address =
            subxt::dynamic::storage(TFGRID_MODULE, FARMS, vec![Value::u128(id.into())]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        if let Ok(farm) = codec::decode_from_bytes::<V115Farm>(r.clone().into()) {
            Ok(Some(farm.into()))
        } else if let Ok(farm) = codec::decode_from_bytes::<V123Farm>(r.clone().into()) {
            Ok(Some(farm.into()))
        } else if let Ok(farm) = codec::decode_from_bytes::<V131Farm>(r.clone().into()) {
            Ok(Some(farm.into()))
        } else if let Ok(farm) = codec::decode_from_bytes::<V141Farm>(r.clone().into()) {
            Ok(Some(farm.into()))
        } else {
            return Err(Error::ErrorDecodingFarm.into());
        }
    }

    /// Get the payout address of the farm referenced by this ID.
    async fn farm_payout_address(
        &self,
        id: u32,
        block: Option<Hash>,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let storage_address = subxt::dynamic::storage(
            TFGRID_MODULE,
            FARM_PAYOUT_V2_ADDRESS,
            vec![Value::u128(id.into())],
        );
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        Ok(Some(codec::decode_from_bytes(r.into())?))
    }

    /// Get the amount of farms on the grid.
    async fn farm_count(&self, block: Option<Hash>) -> Result<u32, Box<dyn std::error::Error>> {
        let storage_address: DynamicStorageAddress<Value> =
            subxt::dynamic::storage(TFGRID_MODULE, FARM_ID, vec![]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch_or_default(&storage_address)
            .await?
            .to_value()?;

        Ok(result.as_u128().map_or(0, |x| x as u32))
    }

    /// Get the node referenced by this ID.
    async fn node(
        &self,
        id: u32,
        block: Option<Hash>,
    ) -> Result<Option<Node>, Box<dyn std::error::Error>> {
        let storage_address =
            subxt::dynamic::storage(TFGRID_MODULE, NODES, vec![Value::u128(id.into())]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        if let Ok(node) = codec::decode_from_bytes::<V115Node>(r.clone().into()) {
            Ok(Some(node.into()))
        } else if let Ok(node) = codec::decode_from_bytes::<V123Node>(r.clone().into()) {
            Ok(Some(node.into()))
        } else if let Ok(node) = codec::decode_from_bytes::<V131Node>(r.clone().into()) {
            Ok(Some(node.into()))
        } else if let Ok(node) = codec::decode_from_bytes::<V141Node>(r.clone().into()) {
            Ok(Some(node.into()))
        } else {
            return Err(Error::ErrorDecodingNode.into());
        }
    }

    /// Get the amount of nodes on the grid.
    async fn node_count(&self, block: Option<Hash>) -> Result<u32, Box<dyn std::error::Error>> {
        let storage_address: DynamicStorageAddress<Value> =
            subxt::dynamic::storage(TFGRID_MODULE, NODE_ID, vec![]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch_or_default(&storage_address)
            .await?
            .to_value()?;

        Ok(result.as_u128().map_or(0, |x| x as u32))
    }

    /// Get the contract referenced by this ID.
    async fn contract(
        &self,
        id: u64,
        block: Option<Hash>,
    ) -> Result<Option<Contract>, Box<dyn std::error::Error>> {
        let storage_address = subxt::dynamic::storage(
            SMART_CONTRACT_MODULE,
            CONTRACTS,
            vec![Value::u128(id.into())],
        );
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        if let Ok(contract) = codec::decode_from_bytes::<V115Contract>(r.clone().into()) {
            Ok(Some(contract.into()))
        } else if let Ok(contract) = codec::decode_from_bytes::<V123Contract>(r.clone().into()) {
            Ok(Some(contract.into()))
        } else if let Ok(contract) = codec::decode_from_bytes::<V131Contract>(r.clone().into()) {
            Ok(Some(contract.into()))
        } else if let Ok(contract) = codec::decode_from_bytes::<V141Contract>(r.clone().into()) {
            Ok(Some(contract.into()))
        } else {
            return Err(Error::ErrorDecodingContract.into());
        }
    }

    /// Get the resources of the contract referenced by this ID.
    async fn contract_resources(
        &self,
        id: u64,
        block: Option<Hash>,
    ) -> Result<Option<ContractResources>, Box<dyn std::error::Error>> {
        let storage_address = subxt::dynamic::storage(
            SMART_CONTRACT_MODULE,
            NODE_CONTRACT_RESOURCES,
            vec![Value::u128(id.into())],
        );
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        if let Ok(contract) = codec::decode_from_bytes::<V115ContractResources>(r.clone().into()) {
            Ok(Some(contract.into()))
        } else if let Ok(contract) =
            codec::decode_from_bytes::<V123ContractResources>(r.clone().into())
        {
            Ok(Some(contract.into()))
        } else if let Ok(contract) =
            codec::decode_from_bytes::<V131ContractResources>(r.clone().into())
        {
            Ok(Some(contract.into()))
        } else if let Ok(contract) =
            codec::decode_from_bytes::<V141ContractResources>(r.clone().into())
        {
            Ok(Some(contract.into()))
        } else {
            return Err(Error::ErrorDecodingContractResources.into());
        }
    }

    /// Get the amount of contracts on the grid.
    async fn contract_count(&self, block: Option<Hash>) -> Result<u64, Box<dyn std::error::Error>> {
        let storage_address: DynamicStorageAddress<Value> =
            subxt::dynamic::storage(SMART_CONTRACT_MODULE, CONTRACT_ID, vec![]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch_or_default(&storage_address)
            .await?
            .to_value()?;

        Ok(result.as_u128().map_or(0, |x| x as u64))
    }

    /// Get the farming policy referenced by this ID.
    async fn farming_policy(
        &self,
        id: u32,
        block: Option<Hash>,
    ) -> Result<Option<FarmPolicy>, Box<dyn std::error::Error>> {
        let storage_address = subxt::dynamic::storage(
            TFGRID_MODULE,
            FARMING_POLICIES,
            vec![Value::u128(id.into())],
        );
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        if let Ok(policy) = codec::decode_from_bytes::<V115FarmingPolicy>(r.clone().into()) {
            Ok(Some(policy.into()))
        } else if let Ok(policy) = codec::decode_from_bytes::<V123FarmingPolicy>(r.clone().into()) {
            Ok(Some(policy.into()))
        } else if let Ok(policy) = codec::decode_from_bytes::<V131FarmingPolicy>(r.clone().into()) {
            Ok(Some(policy.into()))
        } else if let Ok(policy) = codec::decode_from_bytes::<V141FarmingPolicy>(r.clone().into()) {
            Ok(Some(policy.into()))
        } else {
            return Err(Error::ErrorDecodingFarmingPolicy.into());
        }
    }

    /// Get the amount of farming policies on the grid.
    async fn farming_policy_count(
        &self,
        block: Option<Hash>,
    ) -> Result<u32, Box<dyn std::error::Error>> {
        let storage_address: DynamicStorageAddress<Value> =
            subxt::dynamic::storage(TFGRID_MODULE, FARMING_POLICY_ID, vec![]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch_or_default(&storage_address)
            .await?
            .to_value()?;

        Ok(result.as_u128().map_or(0, |x| x as u32))
    }

    /// Get the NodePower for a node
    async fn node_power(
        &self,
        id: u32,
        block: Option<Hash>,
    ) -> Result<Option<NodePower>, Box<dyn std::error::Error>> {
        let storage_address =
            subxt::dynamic::storage(TFGRID_MODULE, NODE_POWER, vec![Value::u128(id.into())]);
        let result = self
            .api
            .storage()
            .at(block)
            .await?
            .fetch(&storage_address)
            .await?;

        if result.is_none() {
            return Ok(None);
        }

        let r: Vec<u8> = result.unwrap().into_encoded().into();

        if let Ok(node_power) = codec::decode_from_bytes::<V131NodePower>(r.into()) {
            Ok(Some(node_power.into()))
        } else if let Ok(node_power) = codec::decode_from_bytes::<V141NodePower>(r.into()) {
            Ok(Some(node_power.into()))
        } else {
            return Err(Error::ErrorDecodingFarmingPolicy.into());
        }
    }
}
