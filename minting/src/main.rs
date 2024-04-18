use crate::{period::Period, violation::Violation};
use blake2::{digest::consts::U32, Blake2b, Digest};
use chrono::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use receipt::{
    CloudUnits, FixupReceipt, MintingReceipt, ResourceRewards, ResourceUnits, ResourceUtilization,
    RetryPayoutReceipt, Reward,
};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    os::unix::prelude::OsStrExt,
    path,
};
use tfchain_client::dynamic::DynamicClient;
use tfchain_client::{
    client::{height_at_timestamp, RuntimeClient},
    types::{
        Contract as ChainContract, ContractData, Farm, FarmPolicy, Location, Node,
        NodeCertification, NodePower, Power, PowerState, Resources, RuntimeEvents, Twin,
    },
};
use tokio::{io::AsyncWriteExt, sync::mpsc};

mod period;
mod receipt;
mod stellar;
mod types;
mod violation;

const RPC_THREADS: usize = 24;
const PRE_FETCH: usize = 5;
/// Currently on TFchain, the weight limit of a block is `2_000_000_000_000`. The vast majority of
/// calls are `UptimeReported` calls and `billContractForBlock` calls. The former has an associated
/// weight of `446_058_000`, while the latter has an associated weight of `780_660_000`. Given that
/// a node sends an uptime report every 40 minutes, the expected amount of uptime reports is (nr
/// nodes * 1.5) per hour, while the expected billContractForBlock call amount per hour is simply
/// nr contracts. Additionally, as part of billing we also have `addNruReports`, which is called
/// once per hour for every contract (technically nodes with contracts call this once per hour and
/// it contains multiple contracts, but the weight scales roughly linearly with the amount of
/// contracts). The weight of one call is `473_727_000`. For calculating the density of these
/// calls, we assume all nodes are up. This gives about 5500 nodes. We also assume 750 contracts
/// (values are taken at the time of writing).
///
/// Since uptime reporting depends on boot time, and contract billing depends on contract creation
/// time, we can use the law of large numbers to model these events as evenly distributed across
/// the available blocks. With a block time of 6 seconds, there are assumed to be 600 blocks per
/// hour for a fully functioning chain. Additionally, if a block is missed, the calls for that
/// block will be added to the next block, recursively, until a block is created. This means that,
/// a block contains all calls for itself and all previously missed blocks.
///
/// For a fully functioning chain, the used weight is 5500/600*780_660_000 +
/// 750/600*(446_058_000+473_727_000). This amounts to 7_156_050_000 + 1_149_731_250 =
/// 8_305_892_250. Considering the previously established block allowed weight of 2E12, that equals
/// to an average of 0.415% of a block. Even if we are pessimistic and assume the actual weight of
/// calls should be double, AND the chain increases 10 times in size of both nodes and contracts,
/// we are only at 8.30% weight, which should not be a problem at all. Considering there are 10
/// block creators, this means we could loose all of them but one and still not have a problem for
/// nodes to push uptimes (note the chain requires more than 2/3 i.e. at least 7 block creators
/// before finalization stalled, so we can assume at most 3 nodes are down at once).
///
/// Considering these constraints, and the invariant that nodes send an uptime report every 40
/// minutes, 1 minute is a sufficiently large grace period: this allows for nodes to _still_ get
/// their uptime reports in the chain even if all nodes on the chain are down except one (which as
/// discussed above is a critical chain situation).
///
/// Numbers are accurate as of 2023-05-08.
const UPTIME_GRACE_PERIOD_SECONDS: i64 = 60; // 1 Minute
/// The maximum allowed clock drift while measuring. Ideally this should be less and this should
/// probably be constrained in the future. We take twice the amount of UPTIME_GRACE_PERIOD_SECONDS
/// for now because we consider that a node can have a skew in one direction of up to this amount,
/// and it would technically be possible to have the same skew in the opposing direction without
/// being considered a validation.
///
/// In practice, a skew which is allowed by the above will happen in one direction and then be
/// fixed later, so technically speaking a copy of the above should be sufficient. To be validated.
// FIXME: This check is faulty as it is way to broad in it's current form, and malfunctioning nodes
// might not be detected.
const CLOCK_SKEW_INTERVAL: i64 = 2 * UPTIME_GRACE_PERIOD_SECONDS;
const NODE_UPTIME_REPORT_INTERVAL_SECONDS: i64 = 60 * 40; // 40 minutes
const GIB: u128 = 1024 * 1024 * 1024;
const ONE_MILL: u128 = 1_000_000;
/// The amount of "units" that make 1 TFT.
const UNITS_PER_TFT: u64 = 10_000_000;
/// The amount of blocks expected in an hour.
const BLOCKS_IN_HOUR: u32 = 10 * 60; // 10 blocks per minute
/// The price of 1 CU related to carbon offset.
/// Value taken from [this]
/// (https://docs.google.com/spreadsheets/d/16uUJEArEb-3aDkHNTlsMpMovyqgLp9xtwzdgjdu-lGw/edit#gid=1395768004)
/// on 01-02-2022.
const CU_CARBON_OFFSET_MUSD: u64 = 354;
/// The price of 1 SU related to carbon offset
/// Value taken from [this]
/// (https://docs.google.com/spreadsheets/d/16uUJEArEb-3aDkHNTlsMpMovyqgLp9xtwzdgjdu-lGw/edit#gid=1395768004)
/// on 01-02-2022.
const SU_CARBON_OFFSET_MUSD: u64 = 122;
/// The address to which the carbon credit tft will be sent.
/// Value provided by Andreas Hartl in a Telegram DM on 03-02-2022
const CARBON_CREDIT_ADDRESS: &str = "GDIJY6K2BBRIRX423ZFUYKKFDN66XP2KMSBZFQSE2PSNDZ6EDVQTRLSU";
/// Address of horizon server to use
//const HORIZON_URL: &str = "https://stellar-mainnet.grid.tf";
const HORIZON_URL: &str = "https://horizon.stellar.org";
/// Maximum amount of seconds a node can be offline because of the power managment feature while
/// still getting rewards.
const MAX_POWER_MANAGER_DOWNTIME: u64 = 60 * 60 * 24;
/// Maximum amount of seconds a node has before it needs to be booted as result of a farmer bot
/// power up request.
const MAX_POWER_MANAGER_BOOT_TIME: i64 = 60 * 30;

#[tokio::main]
async fn main() {
    let mut args = std::env::args();
    // ignore binary name
    args.next();

    let mut log_file = tokio::fs::File::create("minting_log.txt").await.unwrap();

    let period_offset = args.next().unwrap().parse().unwrap();
    if period_offset != 71 {
        panic!("This binary can only be used for period 71");
    }

    let period = Period::at_offset(period_offset);
    let start_ts: i64 = period.start();
    let end_ts: i64 = period.end();
    let wss_url = args.next().unwrap();

    log_file
        .write_all(
            format!(
                "Start minting for period {period_offset} starting at {} and ending at {}\n",
                Utc.timestamp_opt(start_ts, 0).unwrap().to_rfc2822(),
                Utc.timestamp_opt(end_ts, 0).unwrap().to_rfc2822(),
            )
            .as_bytes(),
        )
        .await
        .unwrap();

    let client = DynamicClient::new(&wss_url).await.unwrap();

    println!("Finding start block");
    let start_block = height_at_timestamp(&client, start_ts).await.unwrap();
    println!("Finding end block");
    let end_block = height_at_timestamp(&client, end_ts).await.unwrap();

    log_file
        .write_all(
            format!("Period runs from block {start_block} to block {end_block}\n",).as_bytes(),
        )
        .await
        .unwrap();

    // Grab existing nodes
    let mut nodes: BTreeMap<_, _> = get_nodes(&client, start_block)
        .await
        .unwrap()
        .into_iter()
        .map(|node| {
            (
                node.id,
                MintingNode {
                    id: node.id,
                    farm_id: node.farm_id,
                    twin_id: node.twin_id,
                    resources: node.resources,
                    location: node.location,
                    country: node.country,
                    city: node.city,
                    _created: node.created,
                    certification_type: node.certification,
                    uptime_info: None,
                    boot_time: None,
                    violation: Violation::None,
                    connected: NodeConnected::Old,
                    connection_price: node.connection_price,
                    capacity_consumption: TotalConsumption::default(),
                    virtualized: node.virtualized,
                    farming_policy_id: node.farming_policy_id,
                    power_managed: None,
                    power_manage_boot: None,
                },
            )
        })
        .collect();
    println!("Found {} existing nodes", nodes.len());

    log_file
        .write_all(format!("Loaded {} existing nodes\n", nodes.len()).as_bytes())
        .await
        .unwrap();

    let mut power_states: BTreeMap<_, _> = get_power_states(&client, start_block)
        .await
        .unwrap()
        .into_iter()
        .collect();
    println!("Found {} power states", power_states.len());

    log_file
        .write_all(format!("Loaded {} power states\n", power_states.len()).as_bytes())
        .await
        .unwrap();

    let start_block_hash = client.hash_at_height(Some(start_block)).await.unwrap();
    let start_block_ts = client.timestamp(start_block_hash).await.unwrap() / 1000;

    // Insert missing entries for power state, and update node power managed if it currently is
    // power managed.
    for (node_id, node) in nodes.iter_mut() {
        match power_states.get(&node_id) {
            None => {
                power_states.insert(
                    *node_id,
                    NodePower {
                        state: PowerState::Up,
                        target: Power::Up,
                    },
                );
            }
            Some(NodePower {
                state: PowerState::Down(block),
                target,
            }) => {
                let hash = client.hash_at_height(Some(*block)).await.unwrap();
                let ts = client.timestamp(hash).await.unwrap() / 1000;
                node.power_managed = Some(ts as i64);
                if let Power::Up = target {
                    // Set the powerup request as start timestamp. Technically this is wrong,
                    // however this will be validated properly in the previous period in the
                    // post period checks.
                    node.power_manage_boot = Some(start_block_ts as i64);
                }
            }
            _ => {}
        }
    }

    // Load farms at the end of the period. This means we don't have to parse individual farm
    // events, as we can just fetch the last known state.
    let farms: BTreeMap<_, _> = get_farms(&client, end_block)
        .await
        .unwrap()
        .into_iter()
        .map(|farm| (farm.id, farm))
        .collect();
    println!("Found {} existing farms", farms.len());

    log_file
        .write_all(format!("Loaded {} farms, at the end of the period\n", farms.len()).as_bytes())
        .await
        .unwrap();

    let twins: BTreeMap<_, _> = get_twins(&client, end_block)
        .await
        .unwrap()
        .into_iter()
        .map(|twin| (twin.id, twin))
        .collect();
    println!("Found {} existing twins", twins.len());

    log_file
        .write_all(format!("Loaded {} twins, at the end of the period\n", twins.len()).as_bytes())
        .await
        .unwrap();

    let payout_addresses: BTreeMap<_, _> = get_payout_addresses(&client, &farms, end_block)
        .await
        .unwrap();

    log_file
        .write_all(
            format!(
                "Loaded {} payout addresses, at the end of the period\n",
                payout_addresses.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();

    // Grab existing contracts
    let mut contracts: BTreeMap<_, _> = get_contracts(&client, start_block)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|(contract, resources)| {
            // Namecontract is actually billed once deployed through a node contract.
            if let ContractData::NodeContract(nc) = contract.contract_type {
                Some((
                    contract.contract_id,
                    Contract {
                        _contract_id: contract.contract_id,
                        node_id: nc.node_id,
                        // a report should pop up for this
                        last_report_ts: 0,
                        ips: nc.public_ips,
                        resources,
                    },
                ))
            } else {
                None
            }
        })
        .collect();
    println!("Found {} existing contracts", contracts.len());

    log_file
        .write_all(format!("Loaded {} existing contracts\n", contracts.len()).as_bytes())
        .await
        .unwrap();

    // Get farming policies
    let farming_policies: BTreeMap<_, _> = get_farming_policies(&client, end_block)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|policy| Some((policy.id, policy)))
        .collect();
    println!("Found {} farming policies", farming_policies.len());

    log_file
        .write_all(
            format!(
                "Loaded {} farming policies, at the end of the period\n",
                farming_policies.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();

    println!("Setup block import pipeline");
    let blocks = end_block - start_block + 1;

    let bar = ProgressBar::new(blocks as u64);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("[Time on chain: {msg}] {wide_bar} {pos:>6}/{len:>6} (ETA: {eta_precise})"),
    );
    let mut height = start_block;
    let mut import_queue = block_import(&wss_url, height as usize, end_block as usize).await;
    loop {
        let (block_height, ts, evts) =
            if let Some((block_height, ts, evts)) = import_queue.recv().await {
                (block_height, ts, evts)
            } else {
                panic!("Block import exitted too early");
            };

        log_file
            .write_all(
                format!(
                    "Loaded block {} ({}) containing {} events\n",
                    block_height,
                    Utc.timestamp_opt(ts, 0).unwrap().to_rfc2822(),
                    evts.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        for evt in evts.into_iter() {
            match evt {
                RuntimeEvents::NodeStoredEvent(node) => {
                    nodes.insert(
                        node.id,
                        MintingNode {
                            id: node.id,
                            farm_id: node.farm_id,
                            twin_id: node.twin_id,
                            resources: node.resources,
                            location: node.location,
                            country: node.country,
                            city: node.city,
                            _created: node.created,
                            certification_type: node.certification.clone(),
                            uptime_info: None,
                            boot_time: None,
                            violation: Violation::None,
                            connected: NodeConnected::Current(ts as i64),
                            connection_price: node.connection_price,
                            capacity_consumption: TotalConsumption::default(),
                            virtualized: node.virtualized,
                            farming_policy_id: node.farming_policy_id,
                            power_managed: None,
                            power_manage_boot: None,
                        },
                    );
                    power_states.insert(
                        node.id,
                        NodePower {
                            state: PowerState::Up,
                            target: Power::Up,
                        },
                    );
                    log_file
                        .write_all(format!("New node stored with id {}\n", node.id).as_bytes())
                        .await
                        .unwrap();
                }
                RuntimeEvents::NodeUpdatedEvent(node) => {
                    let old_node = nodes
                        .get_mut(&node.id)
                        .expect("node update of unknown node");
                    old_node.farm_id = node.farm_id;
                    old_node.twin_id = node.twin_id;
                    // update resources, but only lower them in case of dead or removed
                    // hardware. Do not update in case of added hardware as this is currently
                    // unresolved.
                    old_node.resources.cru =
                        std::cmp::min(old_node.resources.cru, node.resources.cru);
                    old_node.resources.mru =
                        std::cmp::min(old_node.resources.mru, node.resources.mru);
                    old_node.resources.hru =
                        std::cmp::min(old_node.resources.hru, node.resources.hru);
                    old_node.resources.sru =
                        std::cmp::min(old_node.resources.sru, node.resources.sru);
                    old_node.location = node.location;
                    old_node.resources = node.resources;
                    old_node.country = node.country;
                    old_node.city = node.city;
                    // Don't care about "create" as that should be fixed anyway
                    // Update certification type. It's technically possible for a node to jump
                    // from DIY to certified and back in the same period, but practically that
                    // should not happen.
                    old_node.certification_type = node.certification.clone();
                    // It is possible that this also causes a node to get a different farming
                    // policy ID.
                    old_node.farming_policy_id = node.farming_policy_id;
                    // Update connection price. This should not happen, but it is here in case
                    // we modify the connection price of the node in place in the future and
                    // emit this generic event when the 5 year fixed time is expired.
                    old_node.connection_price = node.connection_price;
                    // Even though this likely means the node is rebooted, don't mess with
                    // uptime_info. The reboot will be detected in the `NodeUptimeReported`
                    // handler.
                    // This does not change when the node was connected.
                    //
                    // Once a VM, always a VM
                    if node.virtualized {
                        old_node.virtualized = node.virtualized;
                    }

                    log_file
                        .write_all(format!("Node updated with id {}\n", node.id).as_bytes())
                        .await
                        .unwrap();
                }
                RuntimeEvents::NodeUptimeReported(id, current_time, reported_uptime) => {
                    let node = match nodes.get_mut(&id) {
                        Some(node) => node,
                        None => panic!(
                            "can't report uptime for unknown node {} in block {}",
                            id, height
                        ),
                    };

                    // We are power managed and got a request to wake up.
                    match (node.power_managed, node.power_manage_boot) {
                        (Some(time_set_down), Some(boot_request)) => {
                            // Ignore the event if it is sent after the node is supposed to go down,
                            // this will be accounted for once the node starts up again.
                            // For the node to have been properly power managed, it must be booted
                            // after it was set to down.
                            if (current_time - reported_uptime) as i64 > time_set_down {
                                // node got power managed to down
                                let time_delta = current_time as i64 - time_set_down;
                                assert!(time_delta >= 0, "uptime events can't travel back in time");
                                let mut total_uptime =
                                    if let Some((_, _, total_uptime)) = node.uptime_info {
                                        total_uptime
                                    } else {
                                        0
                                    };
                                // Only add uptime if node came back online in time.
                                if time_delta as u64 <= MAX_POWER_MANAGER_DOWNTIME {
                                    // Check and scale to match the actual period start if needed
                                    if time_set_down < start_ts {
                                        total_uptime += (current_time as i64 - start_ts) as u64;
                                        log_file
                                            .write_all(
                                                format!(
                                                    "Added {} seconds of uptime for node {}, scaled in period start\n",
                                                    current_time as i64 - start_ts,
                                                    node.id
                                                )
                                                .as_bytes(),
                                            )
                                            .await
                                            .unwrap();
                                    } else {
                                        total_uptime += time_delta as u64;
                                        log_file
                                            .write_all(
                                                format!(
                                                    "Added {time_delta} seconds of uptime for node {}\n",
                                                    node.id
                                                )
                                                .as_bytes(),
                                            )
                                            .await
                                            .unwrap();
                                    }
                                } else {
                                    log_file
                                        .write_all(format!("Refusing to credit uptime for power managed node {} as the last boot was {time_delta} seconds ago, more than the allowed 24 hours\n", node.id).as_bytes())
                                        .await
                                        .unwrap();
                                }
                                // Speaking of time, node needs to be booted within the allotted time
                                // frame.
                                if (current_time - reported_uptime) as i64 - boot_request
                                    > MAX_POWER_MANAGER_BOOT_TIME
                                {
                                    log_file
                                            .write_all(format!("Detected farmer bot boot violation for node {}, request was done at {} but node only came online at {}\n",
                                                node.id,
                                                Utc.timestamp_opt(boot_request, 0).unwrap().to_rfc2822(),
                                                Utc.timestamp_opt((current_time - reported_uptime) as i64, 0).unwrap().to_rfc2822()
                                            ).as_bytes())
                                            .await
                                            .unwrap();
                                }
                                // Clear the fact that we got power managed, if it is still the case, it
                                // will be set again in the proper event handler.
                                node.power_managed = None;
                                node.power_manage_boot = None;
                                node.uptime_info =
                                    Some((current_time as i64, reported_uptime, total_uptime));
                                // Also mark a boot
                                node.boot_time = Some((
                                    (current_time - reported_uptime) as i64,
                                    current_time as i64,
                                ));
                            } else {
                                log_file
                                .write_all(format!("Ignoring uptime event for node {} as it happened before the node powered down after being requested to do so\n", node.id).as_bytes())
                                .await
                                .unwrap();
                            }
                        }
                        // We are power managed but woke up without boot request. We explicitly ignore this: being
                        // put to sleep by the farmer bot requires a wakeup from the farmer bot. This case also
                        // means nodes just go to sleep anyhow.
                        (Some(_), None) => {
                            log_file
                                .write_all(
                                    format!("Ignoring boot for node {} which is power managed, but did not get a boot request from the farmer bot\n", node.id)
                                        .as_bytes(),
                                )
                                .await
                                .unwrap();
                        }
                        // We got a wakeup request from farmer bot but we are not sleeping due to
                        // the farmer bot. This should not happen.
                        (None, Some(_)) => {
                            log_file
                                .write_all(
                                    format!("Ignoring uptime for node {} after farmer bot asked for a boot while the node was not sleeping as a result of farmer bot\n", node.id)
                                        .as_bytes(),
                                )
                                .await
                                .unwrap();
                        }
                        (None, None) => {
                            if let Some((
                                last_reported_at,
                                last_reported_uptime,
                                mut total_uptime,
                            )) = node.uptime_info
                            {
                                let report_delta = current_time as i64 - last_reported_at;
                                let uptime_delta =
                                    reported_uptime as i64 - last_reported_uptime as i64;
                                // There are quite some situations here. Notice that due to the
                                // blockchain only producing blocks every 6 seconds, and network delay
                                // + a host of other issues, we will allow a node to report uptime with
                                // "grace period" of a minute or so in either direction.
                                //
                                // 1. uptime_delta > report_delta + GRACE_PERIOD. Node is talking
                                //    rubish.
                                if uptime_delta > report_delta + UPTIME_GRACE_PERIOD_SECONDS {
                                    if node.violation.is_none() {
                                        node.violation = Violation::UptimeTooHigh {
                                            previous_uptime: last_reported_uptime,
                                            previous_timestamp: last_reported_at,
                                            reported_uptime,
                                            reported_timestamp: ts,
                                            block_reported: height,
                                        };
                                    }
                                    node.uptime_info =
                                        Some((current_time as i64, reported_uptime, total_uptime));

                                    log_file
                                    .write_all(format!("Node {} reported an uptime increase of {uptime_delta} seconds, while reports are {report_delta} seconds appart\n",node.id,).as_bytes())
                                    .await
                                    .unwrap();
                                    continue;
                                }
                                // 2. The difference in uptime is within reason of the difference in
                                //    report times, i.e. the node is properly reporting.
                                if uptime_delta <= report_delta + UPTIME_GRACE_PERIOD_SECONDS
                                    && uptime_delta >= report_delta - UPTIME_GRACE_PERIOD_SECONDS
                                {
                                    // check skew
                                    if let Some((boot, detected)) = node.boot_time {
                                        let new_boot = (current_time - reported_uptime) as i64;
                                        if (new_boot - boot).abs() >= CLOCK_SKEW_INTERVAL {
                                            // This is a violation
                                            if node.violation.is_none() {
                                                node.violation = Violation::ClockSkew {
                                                    original_boot: boot,
                                                    current_boot: new_boot,
                                                    previous_timestamp: detected,
                                                    reported_timestamp: current_time as i64,
                                                };
                                            }

                                            log_file
                                            .write_all(format!("Node {} has a detected clock skew of {} seconds, more than the allowed {CLOCK_SKEW_INTERVAL} seconds\n",node.id, (new_boot - boot).abs()).as_bytes())
                                            .await
                                            .unwrap();
                                        }
                                    } else {
                                        panic!("node does not have boot time but does have uptime")
                                    }

                                    // It is technically possible for the delta to be less than 0 and
                                    // within the expected time frame. If nodes boot, send uptime, then
                                    // immediately reboot that is possible. In those cases, handle that
                                    // below, as that is the reboot detection.
                                    if uptime_delta > 0 {
                                        // Simply add the uptime delta. If this is too large or low by a
                                        // couple of seconds it will be corrected by the next pings anyhow.
                                        // That being said, we also limit the amount of uptime credit
                                        // to the uptime report interval + grace period, as healthy
                                        // nodes _must_ ping every interval amount of time
                                        let credit = u64::min(
                                            uptime_delta as u64,
                                            (NODE_UPTIME_REPORT_INTERVAL_SECONDS
                                                + UPTIME_GRACE_PERIOD_SECONDS)
                                                as u64,
                                        );
                                        total_uptime += credit;
                                        if credit != uptime_delta as u64 {
                                            log_file
                                            .write_all(format!("credited node {} with {credit} seconds of uptime, less than the reported {uptime_delta} seconds as the gap is too big\n", node.id).as_bytes())
                                            .await
                                            .unwrap();
                                        } else {
                                            log_file
                                            .write_all(format!("credited node {} with {credit} seconds of reported uptime\n", node.id).as_bytes())
                                            .await
                                            .unwrap();
                                        }
                                        node.uptime_info = Some((
                                            current_time as i64,
                                            reported_uptime,
                                            total_uptime,
                                        ));
                                        continue;
                                    }
                                }
                                // 3. The difference in uptime is too low. Again there are multiple
                                //    scenarios. Either way we consider the node rebooted. Depending on
                                //    the reported uptime, the node reports legit uptime, or it reports
                                //    an uptime which is too high.
                                //
                                //    1. Uptime is within bounds.
                                if reported_uptime as i64 <= report_delta {
                                    let credit = u64::min(
                                        reported_uptime,
                                        (NODE_UPTIME_REPORT_INTERVAL_SECONDS
                                            + UPTIME_GRACE_PERIOD_SECONDS)
                                            as u64,
                                    );
                                    total_uptime += credit;
                                    if reported_uptime != credit {
                                        log_file
                                        .write_all(format!("credited node {} with {credit} seconds of uptime after a reboot, less than the reported {reported_uptime} seconds as the gap is too big\n", node.id).as_bytes())
                                        .await
                                        .unwrap();
                                    } else {
                                        log_file
                                        .write_all(format!("credited node {} with {credit} seconds of reported uptime after a reboot\n", node.id).as_bytes())
                                        .await
                                        .unwrap();
                                    }
                                    node.uptime_info =
                                        Some((current_time as i64, reported_uptime, total_uptime));
                                    node.boot_time = Some((
                                        (current_time - reported_uptime) as i64,
                                        current_time as i64,
                                    ));
                                    continue;
                                }
                                //    2. Uptime is actually higher than difference in timestamp, but
                                //       not high enough to be valid. This means the node was
                                //       supposedly rebooted _before_ the previous uptime report,
                                //       meaning either that report is invalid or this report is
                                //       invalid.
                                if reported_uptime > last_reported_uptime {
                                    if node.violation.is_none() {
                                        node.violation = Violation::UptimeTooLow {
                                            previous_uptime: last_reported_uptime,
                                            previous_timestamp: last_reported_at,
                                            reported_uptime,
                                            reported_timestamp: ts,
                                            block_reported: height,
                                        };
                                        log_file
                                        .write_all(format!("Node {} reported uptime of {reported_uptime} seconds, so time would have advanced slower on the node than in the universe\n", node.id).as_bytes())
                                        .await
                                        .unwrap();
                                    }
                                    continue;
                                }
                                //    3. Uptime is too high, this is garbage
                                if node.violation.is_none() {
                                    node.violation = Violation::InvalidReboot {
                                        previous_uptime: last_reported_uptime,
                                        previous_timestamp: last_reported_at,
                                        reported_uptime,
                                        reported_timestamp: ts,
                                        block_reported: height,
                                    };
                                    log_file
                                    .write_all(format!("Node {} reported uptime of {reported_uptime} seconds, so time would have advanced faster on the node than in the universe\n", node.id).as_bytes())
                                    .await
                                    .unwrap();
                                }
                                continue;
                            } else {
                                let period_duration = current_time as i64 - start_ts;
                                // Make sure we don't give more credit than the current length of the
                                // period.
                                // Account for uptime period
                                let up_in_period = u64::min(
                                    std::cmp::min(period_duration as u64, reported_uptime),
                                    (NODE_UPTIME_REPORT_INTERVAL_SECONDS
                                        + UPTIME_GRACE_PERIOD_SECONDS)
                                        as u64,
                                );
                                log_file
                                .write_all(format!("Node {} reported uptime of {reported_uptime} seconds, scaled to {up_in_period} seconds\n", node.id).as_bytes())
                                .await
                                .unwrap();
                                // Save uptime info
                                node.uptime_info =
                                    Some((current_time as i64, reported_uptime, up_in_period));
                                node.boot_time = Some((
                                    (current_time - reported_uptime) as i64,
                                    current_time as i64,
                                ));
                            }
                        }
                    };
                }
                RuntimeEvents::ContractUsedResourcesUpdated(data) => {
                    let contract = match contracts.get_mut(&data.contract_id) {
                        Some(contract) => contract,
                        // Contract needs to exist.
                        None => {
                            panic!("Can't set used resources for contract {} which does not exist in block {}", data.contract_id, height);
                        }
                    };
                    contract.resources = data.used;
                    log_file
                        .write_all(
                            format!(
                                "Update used resources for contract {}\n",
                                contract._contract_id
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                }
                RuntimeEvents::NruConsumptionReceived(data) => {
                    let contract = match contracts.get_mut(&data.contract_id) {
                        Some(contract) => contract,
                        // Contract needs to exist.
                        None => {
                            // If a contract is in grace period, there seem to still be NRU
                            // reports. These may or may not be legit, depending on how zos is set
                            // up. This warrants further investigation, as it would indicate the
                            // network is not properly disconnected, but at this point in time we
                            // will ignore this.
                            continue;
                            // panic!("Can't set used resources for contract {} which does not exist in block {}", data.contract_id, height);
                        }
                    };
                    let node = match nodes.get_mut(&contract.node_id) {
                        Some(node) => node,
                        None => {
                            panic!(
                                "can't process consumption for unknown node {} in block {}",
                                contract.node_id, height
                            )
                        }
                    };
                    // Just to make sure reports are ordered
                    if ts as i64 <= contract.last_report_ts {
                        // Silently ignore reports out of order, we already covered this in an
                        // already processed consumption report. This can happen if the node pushes
                        // a contract consumption report twice.
                        log_file
                            .write_all(
                                format!(
                                    "Ignoring out of order NRU consumption report for contract {} on node {}\n",
                                    contract._contract_id,
                                    node.id,
                                )
                                .as_bytes(),
                            )
                            .await
                            .unwrap();
                        continue;
                    }

                    // If report ts predates start we ignore it.
                    if (ts as i64) < start_ts {
                        log_file
                            .write_all(
                                format!(
                                    "Ignoring NRU consumption report for contract {} on node {} which predates the period start\n",
                                    contract._contract_id,
                                    node.id,
                                )
                                .as_bytes(),
                            )
                            .await
                            .unwrap();
                        continue;
                    }
                    node.capacity_consumption.cru += (contract.resources.cru * data.window) as u128;
                    node.capacity_consumption.mru += (contract.resources.mru * data.window) as u128;
                    node.capacity_consumption.hru += (contract.resources.hru * data.window) as u128;
                    node.capacity_consumption.sru += (contract.resources.sru * data.window) as u128;
                    node.capacity_consumption.ips += contract.ips as u64 * data.window;
                    node.capacity_consumption.nru += data.nru;
                    contract.last_report_ts = ts as i64;
                    log_file
                        .write_all(
                            format!(
                                "Added NRU consumption report for contract {} on node {}\n",
                                contract._contract_id, node.id,
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                }
                RuntimeEvents::ContractCreated(contract) => {
                    if let ContractData::NodeContract(nc) = &contract.contract_type {
                        contracts.insert(
                            contract.contract_id,
                            Contract {
                                _contract_id: contract.contract_id,
                                node_id: nc.node_id,
                                last_report_ts: ts as i64,
                                ips: nc.public_ips,
                                resources: Resources {
                                    hru: 0,
                                    sru: 0,
                                    cru: 0,
                                    mru: 0,
                                },
                            },
                        );
                        log_file
                            .write_all(
                                format!(
                                    "Created contract {} on node {}\n",
                                    contract.contract_id, nc.node_id,
                                )
                                .as_bytes(),
                            )
                            .await
                            .unwrap();
                    };
                }
                RuntimeEvents::PowerTargetChanged(ptc) => {
                    let node_power = match power_states.get_mut(&ptc.node_id) {
                        Some(ps) => ps,
                        None => panic!(
                            "can't change power target for unknown node {} in block {}",
                            ptc.node_id, height
                        ),
                    };
                    log_file
                        .write_all(
                            format!(
                                "Power target changed for node {} from {:?} to {:?}\n",
                                ptc.node_id, node_power.target, ptc.power_target,
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                    // Remember a rising edge here to validate node actually boots.
                    // This is cleared when a node sends an uptime report of a _reboot_. It is
                    // allowed for this to happen if a rising edge is not consumed yet, in which
                    // case the new event is ignored, as we want to measure time from the first
                    // event and it is actually a good idea to send multiple of these if the node
                    // does not react. Of course, we also only want to track this if the node is
                    // currently power managed. While we shouldn't try to boot an online node,
                    // there is no _real_ harm in doing it anyway.
                    if ptc.power_target == Power::Up
                        && matches!(node_power.state, PowerState::Down(_))
                    {
                        if let Some(node) = nodes.get_mut(&ptc.node_id) {
                            // Only remember the first boot request.
                            if node.power_manage_boot.is_none() {
                                node.power_manage_boot = Some(ts);
                                log_file
                                    .write_all(
                                        format!(
                                            "Remembered boot request time for node {}\n",
                                            node.id
                                        )
                                        .as_bytes(),
                                    )
                                    .await
                                    .unwrap();
                            }
                        } else {
                            panic!(
                                "can't change power target for unknown node {} in block {}",
                                ptc.node_id, height
                            );
                        }
                    }
                    node_power.target = ptc.power_target;
                }
                RuntimeEvents::PowerStateChanged(psc) => {
                    let node_power = match power_states.get_mut(&psc.node_id) {
                        Some(ps) => ps,
                        None => panic!(
                            "can't change power state for unknown node {} in block {}",
                            psc.node_id, height
                        ),
                    };
                    log_file
                        .write_all(
                            format!(
                                "Power state changed for node {} from {:?} to {:?}\n",
                                psc.node_id, node_power.state, psc.power_state,
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                    // Add exception to allow node 1 uptime ping once it gets back on which
                    // indicates a reboot.
                    // Also, we only allow this if the target is down as well.
                    if node_power.target == Power::Down {
                        // Only on state transition
                        if node_power.state == PowerState::Up
                            && matches!(psc.power_state, PowerState::Down(_))
                        {
                            // Keep track of this timestamp
                            let node = match nodes.get_mut(&psc.node_id) {
                                Some(node) => node,
                                None => {
                                    panic!("Node must be registered if its power target changes")
                                }
                            };
                            // Either this is Some(timestamp), indicating a previous state
                            // transition which was not followed by an uptime ping once the node
                            // came online. In this case, we ignore that here. This would mean the
                            // node did not come up again.
                            // Otherwise, if None, set the current timestamp as time of going down.
                            if node.power_managed.is_none() {
                                // Also add an implicit uptime.
                                node.power_managed = Some(ts);
                                // While we are at it, credit uptime since last uptime event as
                                // well, as we will use this timestamp as the base for future
                                // uptime calculations.
                                // We don't have to overwrite this since future calculations will
                                // first work on the saved power_managed variable, and will have a
                                // reboot either way.
                                if let Some((last_reported_at, _, mut total_uptime)) =
                                    node.uptime_info
                                {
                                    let delta = ts - last_reported_at;
                                    assert!(
                                        delta >= 0,
                                        "Power state changes can't travel back in time"
                                    );
                                    total_uptime += delta as u64;
                                    // We can set uptime to 0, node will reboot anyway.
                                    node.uptime_info = Some((ts, 0, total_uptime));
                                }
                                log_file
                                    .write_all(
                                        format!(
                                            "Remembered farmer bot shutdown for node {}\n",
                                            node.id
                                        )
                                        .as_bytes(),
                                    )
                                    .await
                                    .unwrap();
                            }
                        }
                    }
                    node_power.state = psc.power_state;
                }
            }
        }

        // finally update progress bar
        bar.set_message(Utc.timestamp_opt(ts as i64, 0).unwrap().to_rfc2822());
        bar.inc(1);

        height += 1;

        if height > end_block {
            break;
        }
    }

    bar.finish();

    println!("Getting uptime info from post period");
    let bar = ProgressBar::new(BLOCKS_IN_HOUR as u64 * 27);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("[Time on chain: {msg}] {wide_bar} {pos:>6}/{len:>6} (ETA: {eta_precise})"),
    );

    // Collect post-period uptime events. Violations don't matter here, those will be handled next
    // period.
    let mut import_queue = block_import(
        &wss_url,
        height as usize,
        end_block as usize + BLOCKS_IN_HOUR as usize * 27,
    )
    .await;
    loop {
        let (block_height, ts, evts) =
            if let Some((block_height, ts, evts)) = import_queue.recv().await {
                (block_height, ts, evts)
            } else {
                panic!("Block import exited too early");
            };
        log_file
            .write_all(
                format!(
                    "Loaded block {} ({}) containing {} events\n",
                    block_height,
                    Utc.timestamp_opt(ts, 0).unwrap().to_rfc2822(),
                    evts.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        for evt in evts.into_iter() {
            match evt {
                RuntimeEvents::NodeUptimeReported(id, current_time, reported_uptime) => {
                    let node = match nodes.get_mut(&id) {
                        Some(node) => node,
                        // This is possible if its an uptime report for a node which came online after
                        // the period ended
                        None => continue,
                    };

                    match (node.power_managed, node.power_manage_boot) {
                        (Some(time_set_down), Some(boot_request)) => {
                            // node got power managed to down
                            let time_delta = current_time as i64 - time_set_down;
                            assert!(time_delta >= 0, "uptime events can't travel back in time");
                            let mut total_uptime = if let Some((
                                last_reported_at,
                                _,
                                total_uptime,
                            )) = node.uptime_info
                            {
                                assert!(
                                    last_reported_at < end_ts,
                                    "there can only be 1 uptime event post period for a power managed node"
                                );
                                total_uptime
                            } else {
                                0
                            };
                            // Only add uptime if node came back online in time.
                            if time_delta as u64 <= MAX_POWER_MANAGER_DOWNTIME {
                                let uptime_diff = end_ts - i64::max(start_ts, time_set_down);
                                assert!(
                                    uptime_diff >= 0,
                                    "uptime event must be sent after node wen't down, chronology"
                                );
                                total_uptime += uptime_diff as u64;
                                log_file
                                .write_all(
                                    format!(
                                        "Added {uptime_diff} seconds of uptime for node {}, for farmer bot boot post period\n",
                                        node.id
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                            }

                            // Speaking of time, node needs to be booted within the allotted time
                            // frame.
                            if (current_time - reported_uptime) as i64 - boot_request
                                > MAX_POWER_MANAGER_BOOT_TIME
                            {
                                log_file
                                        .write_all(format!("Detected farmer bot boot violation for node {}, request was done at {} but node only came online at {}\n",
                                            node.id,
                                            Utc.timestamp_opt(boot_request, 0).unwrap().to_rfc2822(),
                                            Utc.timestamp_opt((current_time - reported_uptime) as i64, 0).unwrap().to_rfc2822()
                                        ).as_bytes())
                                        .await
                                        .unwrap();
                            }
                            // Clear the fact that we got power managed, if it is still the case, it
                            // will be set again in the proper event handler.
                            node.power_managed = None;
                            node.power_manage_boot = None;
                            node.uptime_info =
                                Some((current_time as i64, reported_uptime, total_uptime));
                            // Also mark a boot
                            node.boot_time = Some((
                                (current_time - reported_uptime) as i64,
                                current_time as i64,
                            ));
                        }
                        // We are power managed but woke up without boot request. We explicitly ignore this: being
                        // put to sleep by the farmer bot requires a wakeup from the farmer bot. This case also
                        // means nodes just go to sleep anyhow.
                        (Some(_), None) => {
                            log_file
                                .write_all(
                                    format!("Ignoring boot for node {} which is power managed, but did not get a boot request from the farmer bot in the period\n", node.id)
                                        .as_bytes(),
                                )
                                .await
                                .unwrap();
                        }
                        // We got a wakeup request from farmer bot but we are not sleeping due to
                        // the farmer bot. This should not happen.
                        (None, Some(_)) => {
                            log_file
                                .write_all(
                                    format!("Ignoring uptime for node {} after farmer bot asked for a boot while the node was not sleeping as a result of farmer bot\n", node.id)
                                        .as_bytes(),
                                )
                                .await
                                .unwrap();
                        }
                        (None, None) => {
                            if let Some((
                                last_reported_at,
                                last_reported_uptime,
                                mut total_uptime,
                            )) = node.uptime_info
                            {
                                // only collect 1 uptime event after the period ended
                                if last_reported_at >= end_ts {
                                    continue;
                                }
                                let report_delta = current_time as i64 - last_reported_at;
                                let uptime_delta =
                                    reported_uptime as i64 - last_reported_uptime as i64;
                                let delta_in_period = end_ts - last_reported_at;
                                // There are quite some situations here. Notice that due to the
                                // blockchain only producing blocks every 6 seconds, and network delay
                                // + a host of other issues, we will allow a node to report uptime with
                                // "grace period" of a minute or so in either direction.
                                //
                                // 1. uptime_delta > report_delta + GRACE_PERIOD. Node is talking
                                //    rubish.
                                if uptime_delta > report_delta + UPTIME_GRACE_PERIOD_SECONDS {
                                    // We need to register the violation here as we won't be able to
                                    // next period (since we don't scrape points from before the period
                                    // atm).
                                    if node.violation.is_none() {
                                        node.violation = Violation::UptimeTooHigh {
                                            previous_uptime: last_reported_uptime,
                                            previous_timestamp: last_reported_at,
                                            reported_uptime,
                                            reported_timestamp: ts,
                                            block_reported: height,
                                        };
                                    }
                                    node.uptime_info =
                                        Some((current_time as i64, reported_uptime, total_uptime));
                                    log_file
                                    .write_all(format!("Node {} reported an uptime increase of {uptime_delta} seconds, while reports are {report_delta} seconds appart\n",node.id,).as_bytes())
                                    .await
                                    .unwrap();
                                    continue;
                                }
                                // 2. The difference in uptime is within reason of the difference in
                                //    report times, i.e. the node is properly reporting.
                                if uptime_delta <= report_delta + UPTIME_GRACE_PERIOD_SECONDS
                                    && uptime_delta >= report_delta - UPTIME_GRACE_PERIOD_SECONDS
                                {
                                    // check skew
                                    if let Some((boot, detected)) = node.boot_time {
                                        let new_boot = (current_time - reported_uptime) as i64;
                                        if (new_boot - boot).abs() >= CLOCK_SKEW_INTERVAL {
                                            // This is a violation
                                            if node.violation.is_none() {
                                                node.violation = Violation::ClockSkew {
                                                    original_boot: boot,
                                                    current_boot: new_boot,
                                                    previous_timestamp: detected,
                                                    reported_timestamp: current_time as i64,
                                                };
                                            }
                                            log_file
                                            .write_all(format!("Node {} has a detected clock skew of {} seconds, more than the allowed {CLOCK_SKEW_INTERVAL} seconds\n",node.id, (new_boot - boot).abs()).as_bytes())
                                            .await
                                            .unwrap();
                                        }
                                    } else {
                                        panic!("node does not have boot time but does have uptime")
                                    }

                                    // It is technically possible for the delta to be less than 0 and
                                    // within the expected time frame. If nodes boot, send uptime, then
                                    // immediately reboot that is possible. In those cases, handle that
                                    // below, as that is the reboot detection.
                                    if uptime_delta > 0 {
                                        // Simply add the uptime delta. If this is too large or low by a
                                        // couple of seconds it will be corrected by the next pings anyhow.
                                        //
                                        // Make sure we don't add too much based on the period.
                                        let credit = u64::min(
                                            delta_in_period as u64,
                                            (NODE_UPTIME_REPORT_INTERVAL_SECONDS
                                                + UPTIME_GRACE_PERIOD_SECONDS)
                                                as u64,
                                        );
                                        total_uptime += credit;
                                        if credit != delta_in_period as u64 {
                                            log_file
                                            .write_all(format!("credited node {} with {credit} seconds of uptime, less than the reported {delta_in_period} seconds as the gap is too big\n", node.id).as_bytes())
                                            .await
                                            .unwrap();
                                        } else {
                                            log_file
                                            .write_all(format!("credited node {} with {credit} seconds of reported uptime\n", node.id).as_bytes())
                                            .await
                                            .unwrap();
                                        }
                                        node.uptime_info = Some((
                                            current_time as i64,
                                            reported_uptime,
                                            total_uptime,
                                        ));
                                        continue;
                                    }
                                }
                                // 3. The difference in uptime is too low. Again there are multiple
                                //    scenarios. Either way we consider the node rebooted. Depending on
                                //    the reported uptime, the node reports legit uptime, or it reports
                                //    an uptime which is too high.
                                //
                                //    1. Uptime is within bounds.
                                if reported_uptime as i64 <= report_delta {
                                    // Account for the fact that we are actually out of the period
                                    let out_of_period = current_time - end_ts as u64;
                                    if out_of_period < reported_uptime {
                                        let credit = u64::min(
                                            reported_uptime - out_of_period,
                                            (NODE_UPTIME_REPORT_INTERVAL_SECONDS
                                                + UPTIME_GRACE_PERIOD_SECONDS)
                                                as u64,
                                        );
                                        total_uptime += credit;
                                        if (reported_uptime - out_of_period) != credit {
                                            log_file
                                        .write_all(format!("credited node {} with {credit} seconds of uptime after a reboot, less than the reported {} seconds as the gap is too big\n", node.id, reported_uptime - out_of_period).as_bytes())
                                        .await
                                        .unwrap();
                                        } else {
                                            log_file
                                        .write_all(format!("credited node {} with {credit} seconds of reported uptime after a reboot\n", node.id).as_bytes())
                                        .await
                                        .unwrap();
                                        }
                                    }
                                    node.uptime_info =
                                        Some((current_time as i64, reported_uptime, total_uptime));
                                    node.boot_time = Some((
                                        (current_time - reported_uptime) as i64,
                                        current_time as i64,
                                    ));
                                    continue;
                                }
                                //    2. Uptime is actually higher than difference in timestamp, but
                                //       not high enough to be valid. This means the node was
                                //       supposedly rebooted _before_ the previous uptime report,
                                //       meaning either that report is invalid or this report is
                                //       invalid.
                                if reported_uptime > last_reported_uptime {
                                    if node.violation.is_none() {
                                        node.violation = Violation::UptimeTooLow {
                                            previous_uptime: last_reported_uptime,
                                            previous_timestamp: last_reported_at,
                                            reported_uptime,
                                            reported_timestamp: ts,
                                            block_reported: height,
                                        };
                                        log_file
                                        .write_all(format!("Node {} reported uptime of {reported_uptime} seconds, so time would have advanced slower on the node than in the universe\n", node.id).as_bytes())
                                        .await
                                        .unwrap();
                                    }
                                    continue;
                                }
                                //    3. Uptime is too high, this is garbage
                                if node.violation.is_none() {
                                    node.violation = Violation::InvalidReboot {
                                        previous_uptime: last_reported_uptime,
                                        previous_timestamp: last_reported_at,
                                        reported_uptime,
                                        reported_timestamp: ts,
                                        block_reported: height,
                                    };
                                    log_file
                                    .write_all(format!("Node {} reported uptime of {reported_uptime} seconds, so time would have advanced faster on the node than in the universe\n", node.id).as_bytes())
                                    .await
                                    .unwrap();
                                    continue;
                                }

                                // We should have handled all cases. Make this explicit here.
                                unreachable!();
                            }
                        }
                    }
                }
                _ => {
                    // Don't care here
                }
            }
        }

        bar.set_message(Utc.timestamp_opt(ts as i64, 0).unwrap().to_rfc2822());
        bar.inc(1);

        height += 1;

        if height > end_block + BLOCKS_IN_HOUR * 27 {
            break;
        }
    }
    bar.finish_and_clear();

    // At this point we are done fetching events. Note that for the case of power manager boot
    // requests, we haven't checked the case where the node does not respond at all. We already
    // fetched a days worth of blocks after the period ended, and don't keep track of power on
    // requests there. So any leftover requests here are already a day old, which is way too much.
    // So if any node has an outstanding power on request here, stick them with a violation.
    for (_, node) in nodes.iter_mut() {
        if let Some(boot_request) = node.power_manage_boot {
            // Ignore if this is the same as start, no need to slap a violation on what is likely a
            // dead node.
            if boot_request == start_block_ts as i64 {
                log_file
                    .write_all(format!("Not giving node {} a slow boot violation since it never tried to boot in the first place\n",
                        node.id,
                    ).as_bytes())
                    .await
                    .unwrap();
                continue;
            }
            log_file
                    .write_all(format!("Detected farmer bot boot violation for node {}, request was done at {} but node never booted\n",
                        node.id,
                        Utc.timestamp_opt(boot_request, 0).unwrap().to_rfc2822(),
                    ).as_bytes())
                    .await
                    .unwrap();
        }
    }

    // Check twin relays and public keys
    for (_, node) in nodes.iter_mut() {
        // We only care for nodes which are online.
        if node.uptime_info.is_none() {
            continue;
        }
        let twin = if let Some(twin) = twins.get(&node.twin_id) {
            twin
        } else {
            // This should not happen, but still catch it
            if node.violation.is_none() {
                node.violation = Violation::MissingTwin;
            }
            log_file
                .write_all(format!("Node {} ended period without twin\n", node.id,).as_bytes())
                .await
                .unwrap();
            continue;
        };
        let has_relay = match twin.relay {
            None => false,
            Some(ref s) if s.is_empty() => false,
            _ => true,
        };
        if !has_relay && node.violation.is_none() {
            node.violation = Violation::MissingRelay;
            log_file
                .write_all(
                    format!("Node {} ended period without twin relay set\n", node.id,).as_bytes(),
                )
                .await
                .unwrap();
        }
        if let Some(ref pk) = twin.pk {
            // Secp256k1 public key size is 33 bytes in compressed form
            if pk.len() != 33 && node.violation.is_none() {
                node.violation = Violation::InvalidPublicKey;
                log_file
                    .write_all(
                        format!(
                            "Node {} ended period with invalid public key set on twin\n",
                            node.id,
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        }
    }

    let mut overview_file = std::fs::File::create("overview.csv").unwrap();

    writeln!(overview_file,"node id,twin id,farm name (farm id),period start,period end,measured uptime,CU,SU,NU,USD reward,TFT reward,TFT price on connect,carbon offset USD generated,carbon offset TFT generated,cru,cru used,mru,mru used,hru,hru used,sru,sru used,IP used,DIY state,Virtualized,violation,stellar address").unwrap();
    for (_, node) in nodes {
        // generate receipt
        let receipt = node.receipt(period, &farms, &payout_addresses, &farming_policies);

        let node_period = receipt.period;
        let node_start = Utc.timestamp_opt(node_period.start(), 0).unwrap();
        let node_end = Utc.timestamp_opt(node_period.end(), 0).unwrap();
        let CloudUnits { cu, su, nu } = receipt.cloud_units;
        let Reward { musd, tft } = receipt.reward;
        let Reward {
            musd: co_musd,
            tft: co_tft,
        } = receipt.carbon_offset;
        let ResourceUtilization {
            cru: cru_used,
            mru: mru_used,
            hru: hru_used,
            sru: sru_used,
            ip: ip_used,
        } = receipt.resource_utilization;
        let farm = if let Some(farm) = farms.get(&receipt.farm_id) {
            farm
        } else {
            println!(
                "node {} is in farm {} which does not exist anymore",
                receipt.node_id, receipt.farm_id
            );
            continue;
        };
        let stellar_address = if let Some(stellar_address) = payout_addresses.get(&receipt.farm_id)
        {
            &stellar_address
        } else {
            ""
        };
        writeln!(overview_file,
            "{},{},{} ({}),{},{},{},{},{},{},{} $,{},{} $,{} $,{} TFT,{},{:.2}%,{},{:.2}%,{},{:.2}%,{},{:.2}%,{:.2} hours,{},{},{},{}",
            node.id,
            node.twin_id,
            farm.name,
            node.farm_id,
            node_start,
            node_end,
            receipt.measured_uptime,
            format_args!("{:.6}", cu ),
            format_args!("{:.6}", su ),
            format_args!("{:.6}", nu ),
            format_args!("{}.{:03}", musd / 1_000, musd % 1_000),
            format_args!("{}.{:07}", tft / UNITS_PER_TFT, tft % UNITS_PER_TFT),
            format_args!(
                "{}.{:03}",
                receipt.tft_connection_price / 1_000,
                receipt.tft_connection_price % 1_000
            ),
            format_args!("{}.{:03}", co_musd / 1_000, co_musd % 1_000),
            format_args!("{}.{:07}", co_tft / UNITS_PER_TFT, co_tft % UNITS_PER_TFT),
            receipt.resource_units.cru,
            cru_used,
            receipt.resource_units.mru,
            mru_used,
            receipt.resource_units.hru,
            hru_used,
            receipt.resource_units.sru,
            sru_used,
            ip_used,
            receipt.node_type,
            node.virtualized,
            node.violation,
            stellar_address,
        ).unwrap();
    }
}

enum NodeConnected {
    /// Node was connected in a previous period.
    Old,
    /// Node was connected in the current period, first seen at the specified timestamp.
    Current(i64),
}

struct MintingNode {
    id: u32,
    farm_id: u32,
    twin_id: u32,
    resources: Resources,
    location: Location,
    country: String,
    city: String,
    _created: u64,
    certification_type: NodeCertification,
    // (last ping, last reported uptime, total uptime).
    uptime_info: Option<(i64, u64, u64)>,
    // (boot time, original boot time record).
    boot_time: Option<(i64, i64)>,
    violation: Violation,
    connected: NodeConnected,
    // TFT price expressed in USD at time of connection. Price is expressed in mUSD (3 digits
    // precision). I.e. 1 USD => 1000.
    connection_price: u32,
    // capacity consumed by workloads over a period.
    capacity_consumption: TotalConsumption,
    virtualized: bool,
    farming_policy_id: u32,
    // Timestamp the node changed powerstate to down, before a new uptime was posted.
    power_managed: Option<i64>,
    /// Time the last power manage target changed to up. We keep track of this to make sure we
    /// always have a node go up after target is set to up (i.e. farmerbot powers on a node).
    /// Cleared when node boots.
    power_manage_boot: Option<i64>,
}

impl MintingNode {
    /// Compute the CU, SU and NU for the node. The result is expressed in a "permill" way. So the
    /// actual CU, SU and NU are obtained by dividing the results by 1_000_000.
    ///
    /// In order for this to be accurate, the data about network and IP usage needs to already have
    /// been aggregated on the node object.
    ///
    /// Calculation taken from [the
    /// wiki](https://library.threefold.me/info/threefold#/tfgrid/farming/threefold__resource_units_calc_cloudunits)
    /// on 31-01-2022 as follows:
    ///   CU: MIN(cru * 4 / 2, (mru - 1) / 4, sru / 50)
    ///   SU: hru / 1200 + sru * 0.8 / 200
    ///   NU: gigabytes of public traffic reported
    fn cloud_units_permill(&self) -> (u64, u64, u64) {
        // Calculate CU first. Mru and sru are in bytes, but are expressed in GB in the formula.
        // Rather than dividing first, we multiply cru first, then take the MIN, and finally
        // divide. This eliminates the issue of rounding errors _BEFORE_ the MIN. Also multiply by
        // 1000000 so we have precision without working with floats.
        //
        // MIN is associative.
        let cu_intermediate = std::cmp::min(
            self.resources.cru as u128 * 2 * GIB * ONE_MILL,
            (self.resources.mru as u128 - 1 * GIB) * ONE_MILL / 4,
        );
        let cu = std::cmp::min(cu_intermediate, self.resources.sru as u128 * ONE_MILL / 50);
        let su = self.resources.hru as u128 * ONE_MILL / 1200
            + self.resources.sru as u128 * ONE_MILL / 250;
        let nu = self.capacity_consumption.nru as u128 * ONE_MILL;
        ((cu / GIB) as u64, (su / GIB) as u64, (nu / GIB) as u64)
    }

    /// Calculate the USD payout of the node in a minting period based on its cloud units. The
    /// payout is expressed in mUSD, i.e. 1 USD == 1000.
    ///
    /// In order for this to be accurate, the data about network and IP usage needs to already have
    /// been aggregated on the node object.
    ///
    /// Payout =
    ///     CU * CU_REWARD
    ///     + SU * SU_REWARD
    ///     + NU used * NU REWARD
    ///     + IP used * IP REWARD
    ///
    /// Additionally, "certified" nodes get 25% extra.
    ///
    /// A virtualized node (i.e. zos running in VM) won't get anything.
    fn node_payout_musd(&self, farming_policies: &BTreeMap<u32, FarmPolicy>) -> u64 {
        if self.virtualized || self.violation.is_some() {
            return 0;
        }
        let policy = farming_policies.get(&self.farming_policy_id).unwrap();
        let (cu, su, nu) = self.cloud_units_permill();
        let cu_reward = cu * policy.cu as u64;
        let su_reward = su * policy.su as u64;
        let nu_reward = nu * policy.nu as u64;
        // Recall that IP usage is actually in seconds. Multiply the seconds of IP usage with the
        // hourly reward, then divide by 3600 seconds/hour. This prevents issues with low usage.
        let ip_reward = self.capacity_consumption.ips * policy.ipv4 as u64 / 3600;
        let base_payout = (cu_reward + su_reward + nu_reward) / ONE_MILL as u64 + ip_reward;
        // TODO: remove once Titans have policy id 2
        if matches!(self.certification_type, NodeCertification::Certified)
            && self.farming_policy_id == 1
        {
            base_payout * 5 / 4
        } else {
            base_payout
        }
    }

    /// Calculate the TFT payout of the node in the minting period based on its cloud units and
    /// connection price. The payout is expressed in "units", where 1 TFT == 10_000_000 units.
    /// This also acconts for measured NU and Ip usage.
    fn node_payout_tft_units(&self, farming_policies: &BTreeMap<u32, FarmPolicy>) -> u64 {
        // connection price is in mUSD.
        self.node_payout_musd(farming_policies) * UNITS_PER_TFT / self.connection_price as u64
    }

    /// Calculate the amount of mUSD generated by the node for carbon offset.
    ///
    /// This is solely based on the CU and SU, therefore it can be calculated from just the node
    /// definition without aggregating the blocks of a period.
    ///
    /// A virtualized node is garbage so that does not receive anything.
    fn node_carbon_musd(&self) -> u64 {
        if self.virtualized || self.violation.is_some() {
            return 0;
        }
        let (cu, su, _) = self.cloud_units_permill();
        let cu_payout = cu * CU_CARBON_OFFSET_MUSD;
        let su_payout = su * SU_CARBON_OFFSET_MUSD;

        (cu_payout + su_payout) / ONE_MILL as u64
    }

    /// Calculate the TFT payout generated by the node towards carbon offset.
    fn node_carbon_tft_units(&self) -> u64 {
        self.node_carbon_musd() * UNITS_PER_TFT / self.connection_price as u64
    }

    /// Get the real period for the node given an observed period.
    ///
    /// If the node only came online in the observed period, then a period which starts at the
    /// moment of connection will be returned.
    fn real_period(&self, mut observed_period: Period) -> Period {
        if let NodeConnected::Current(ts) = self.connected {
            observed_period.scale_start(ts);
        };
        observed_period
    }

    fn receipt(
        &self,
        period: Period,
        farms: &BTreeMap<u32, Farm>,
        payout_addresses: &BTreeMap<u32, String>,
        farming_policies: &BTreeMap<u32, FarmPolicy>,
    ) -> MintingReceipt {
        let uptime = if let Some((_, _, uptime)) = self.uptime_info {
            uptime
        } else {
            0
        };
        let farm = farms.get(&self.farm_id).unwrap();
        let (cu, su, nu) = self.cloud_units_permill();
        let cu = cu as f64 / ONE_MILL as f64;
        let su = su as f64 / ONE_MILL as f64;
        let nu = nu as f64 / ONE_MILL as f64;
        let cru_used = (self.capacity_consumption.cru / period.duration() as u128) as u64;
        let mru_used = (self.capacity_consumption.mru / period.duration() as u128) as u64;
        let hru_used = (self.capacity_consumption.hru / period.duration() as u128) as u64;
        let sru_used = (self.capacity_consumption.sru / period.duration() as u128) as u64;
        let (musd, tft) = self.scaled_payout(period, farming_policies);
        let (carbon_musd, carbon_tft) = self.scaled_carbon_payout(period);
        let payout_address = match payout_addresses.get(&self.farm_id) {
            Some(address) => address,
            None => "",
        };
        let policy = farming_policies.get(&self.farming_policy_id).unwrap();
        MintingReceipt {
            period: self.real_period(period),
            node_id: self.id,
            twin_id: self.twin_id,
            farm_id: self.farm_id,
            farm_name: farm.name.clone(),
            stellar_payout_address: payout_address.to_string(),
            measured_uptime: uptime,
            // TODO: revert once fixed on chain (period 54 presumably)
            // tft_connection_price: self.connection_price as u64,
            tft_connection_price: 80,
            cloud_units: CloudUnits { cu, su, nu },
            resource_units: ResourceUnits {
                cru: self.resources.cru as f64,
                mru: self.resources.mru as f64 / GIB as f64,
                sru: self.resources.sru as f64 / GIB as f64,
                hru: self.resources.hru as f64 / GIB as f64,
            },
            resource_utilization: ResourceUtilization {
                cru: if self.resources.cru > 0 {
                    cru_used as f64 * 100. / self.resources.cru as f64
                } else {
                    0.
                },
                mru: if self.resources.mru > 0 {
                    mru_used as f64 * 100. / self.resources.mru as f64
                } else {
                    0.
                },
                sru: if self.resources.sru > 0 {
                    sru_used as f64 * 100. / self.resources.sru as f64
                } else {
                    0.
                },
                hru: if self.resources.hru > 0 {
                    hru_used as f64 * 100. / self.resources.hru as f64
                } else {
                    0.
                },
                ip: self.capacity_consumption.ips as f64 / 3600.,
            },
            reward: Reward { musd, tft },
            carbon_offset: Reward {
                musd: carbon_musd,
                tft: carbon_tft,
            },
            node_type: match self.certification_type {
                NodeCertification::Diy => "DIY".into(),
                NodeCertification::Certified => "CERTIFIED".into(),
            },
            farming_policy_id: self.farming_policy_id,
            resource_rewards: ResourceRewards {
                cu: policy.cu as u64,
                su: policy.su as u64,
                nu: policy.nu as u64,
                ipv4: policy.ipv4 as u64,
            },
        }
    }

    /// Get the adjusted uptime of the node
    fn uptime(&self, period: Period) -> u64 {
        let period_duration = self.real_period(period).duration();
        if let Some((_, _, uptime)) = self.uptime_info {
            std::cmp::min(uptime, period_duration)
        } else {
            0
        }
    }

    /// Get the payout for a node in mUSD and units TFT for a period. This accounts for scaled
    /// period due to connection time, and SLA.
    ///
    /// Payout is linear to node uptime in the period.
    fn scaled_payout(
        &self,
        period: Period,
        farming_policies: &BTreeMap<u32, FarmPolicy>,
    ) -> (u64, u64) {
        if let Some((_, _, uptime)) = self.uptime_info {
            // Calculate uptime with 0.001% precision by upscaling with factor 1_000.
            // We don't scale the period since the linear payment cancels out eventually.
            let mut uptime_percentage = uptime * 1_000 / period.duration();
            // Sanity check
            if uptime_percentage > 1_000 {
                uptime_percentage = 1_000;
            }

            // Scale payouts for now, remember to divide by the upscale.
            if self.farming_policy_id == 1
                || self.farming_policy_id == 2
                    // Convoluted check because new nodes on testnet seem to get farming policy 3,
                    // without having to add explicit network selectors.
                || (self.farming_policy_id == 3
                    && if let Some(policy) = farming_policies.get(&3) {
                        !policy.default && !policy.immutable && policy.minimal_uptime == 95
                    } else {
                        false
                    })
            {
                (
                    self.node_payout_musd(farming_policies) * uptime_percentage / 1_000,
                    self.node_payout_tft_units(farming_policies) * uptime_percentage / 1_000,
                )
            } else {
                // Not the default policy, enforce the minimal uptime
                let policy = farming_policies.get(&self.farming_policy_id).unwrap();
                if uptime_percentage < policy.minimal_uptime as u64 {
                    (0, 0)
                } else {
                    (
                        self.node_payout_musd(farming_policies),
                        self.node_payout_tft_units(farming_policies),
                    )
                }
            }
        } else {
            (0, 0)
        }
    }

    /// Get the carbon payout for a node in mUSD and units TFT for a period. This scales linearly with
    /// uptime of the node. Importantly, this does not scale with the connection time, as carbon
    /// untis are expressed for a whole duration.
    fn scaled_carbon_payout(&self, period: Period) -> (u64, u64) {
        let musd = self.node_carbon_musd();
        let tft = self.node_carbon_tft_units();
        let period_duration = period.duration();
        let uptime = self.uptime(period);

        (
            musd * uptime / period_duration,
            tft * uptime / period_duration,
        )
    }
}

struct Contract {
    _contract_id: u64,
    node_id: u32,
    // timestamp of last report. If not set, time when the contract was created.
    last_report_ts: i64,
    ips: u32,
    // Resources set on chain
    resources: Resources,
}

#[derive(Default)]
struct TotalConsumption {
    // cru mru hru sru and ips is value * time i.e. unit seconds
    cru: u128,
    sru: u128,
    hru: u128,
    mru: u128,
    ips: u64,
    nru: u64,
}

pub async fn get_nodes(
    client: &dyn RuntimeClient,
    block: u32,
) -> Result<Vec<Node>, Box<dyn std::error::Error>> {
    let hash = client.hash_at_height(Some(block)).await?;
    let node_count = client.node_count(hash).await?;
    let mut nodes = Vec::new();
    for i in 1..=node_count {
        if let Some(node) = client.node(i, hash).await? {
            nodes.push(node);
        }
    }
    Ok(nodes)
}

pub async fn get_twins(
    client: &dyn RuntimeClient,
    block: u32,
) -> Result<Vec<Twin>, Box<dyn std::error::Error>> {
    let hash = client.hash_at_height(Some(block)).await?;
    let twin_count = client.twin_count(hash).await?;
    let mut twins = Vec::new();
    for i in 1..=twin_count {
        if let Some(twin) = client.twin(i, hash).await? {
            twins.push(twin);
        }
    }
    Ok(twins)
}

pub async fn get_farms(
    client: &dyn RuntimeClient,
    block: u32,
) -> Result<Vec<Farm>, Box<dyn std::error::Error>> {
    let hash = client.hash_at_height(Some(block)).await?;
    let farm_count = client.farm_count(hash).await?;
    let mut farms = Vec::new();
    for i in 1..=farm_count {
        if let Some(farm) = client.farm(i, hash).await? {
            farms.push(farm);
        }
    }
    Ok(farms)
}

pub async fn get_payout_addresses(
    client: &dyn RuntimeClient,
    farms: &BTreeMap<u32, Farm>,
    block: u32,
) -> Result<BTreeMap<u32, String>, Box<dyn std::error::Error>> {
    let hash = client.hash_at_height(Some(block)).await?;
    let mut addresses = BTreeMap::new();
    for (&id, _) in farms {
        match client.farm_payout_address(id, hash).await? {
            Some(a) => {
                addresses.insert(id, a);
            }
            None => continue,
        }
    }
    Ok(addresses)
}

pub async fn get_contracts(
    client: &dyn RuntimeClient,
    block: u32,
) -> Result<Vec<(ChainContract, Resources)>, Box<dyn std::error::Error>> {
    let hash = client.hash_at_height(Some(block)).await?;
    let contract_count = client.contract_count(hash).await?;
    let mut contracts = Vec::new();
    for i in 1..=contract_count {
        if let Some(contract) = client.contract(i, hash).await? {
            if let Some(contract_resources) = client.contract_resources(i, hash).await? {
                contracts.push((contract, contract_resources.used));
            } else {
                contracts.push((
                    contract,
                    Resources {
                        hru: 0,
                        sru: 0,
                        cru: 0,
                        mru: 0,
                    },
                ));
            }
        }
    }
    Ok(contracts)
}

pub async fn get_farming_policies(
    client: &dyn RuntimeClient,
    block: u32,
) -> Result<Vec<FarmPolicy>, Box<dyn std::error::Error>> {
    let hash = client.hash_at_height(Some(block)).await?;
    let policy_count = client.farming_policy_count(hash).await?;
    let mut policies = Vec::new();
    for i in 1..=policy_count {
        if let Some(farm_policy) = client.farming_policy(i, hash).await? {
            policies.push(farm_policy);
        }
    }
    Ok(policies)
}

pub async fn get_power_states(
    client: &dyn RuntimeClient,
    block: u32,
) -> Result<Vec<(u32, NodePower)>, Box<dyn std::error::Error>> {
    let hash = client.hash_at_height(Some(block)).await?;
    let node_count = client.node_count(hash).await?;
    let mut power_states = Vec::new();
    for i in 1..=node_count {
        if let Some(ps) = client.node_power(i, hash).await? {
            power_states.push((i, ps));
        }
    }
    Ok(power_states)
}

async fn block_import(
    wss_url: &str,
    start: usize,
    end: usize,
) -> mpsc::Receiver<(u32, i64, Vec<RuntimeEvents>)> {
    let mut t_rec = Vec::with_capacity(RPC_THREADS);
    for i in 0..RPC_THREADS {
        let client = DynamicClient::new(&wss_url).await.unwrap();
        let (tx, rx) = mpsc::channel(PRE_FETCH);
        let mut height = start + i;
        tokio::task::spawn(async move {
            loop {
                let hash = client.hash_at_height(Some(height as u32)).await.unwrap();
                let evts = client.events(hash).await.unwrap();
                let ts = client.timestamp(hash).await.unwrap() / 1000;
                if let Err(e) = tx.send((height as u32, ts as i64, evts)).await {
                    panic!("{e}");
                }
                height += RPC_THREADS;

                if height > end {
                    break;
                }
            }
        });
        t_rec.push(rx);
    }

    let (tx, rx) = mpsc::channel(5);
    tokio::task::spawn(async move {
        let mut i = 0;
        let l = t_rec.len();
        loop {
            if let Some(r) = t_rec[i % l].recv().await {
                if let Err(e) = tx.send(r).await {
                    panic!("{e}");
                }
            } else {
                break;
            }
            i += 1;
        }
    });

    rx
}
