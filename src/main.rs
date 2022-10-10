use crate::period::Period;
use blake2::{digest::consts::U32, Blake2b, Digest};
use chrono::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use receipt::{
    CloudUnits, FixupReceipt, MintingReceipt, ResourceRewards, ResourceUnits, ResourceUtilization,
    RetryPayoutReceipt, Reward,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    mem,
    os::unix::prelude::OsStrExt,
    path,
    sync::mpsc,
};
use tfchain_client::{
    client::{Client, MultiSignature, Pair, SharedClient},
    events::{SmartContractEvent, TFGridEvent, TfchainEvent},
    window::{Network, Window},
};
use types::{
    BlockNumber, ContractData, Farm, FarmingPolicy, Location, NodeCertification, Resources,
};

mod period;
mod receipt;
mod stellar;
mod types;

const RPC_THREADS: usize = 100;
const PRE_FETCH: usize = 5;
const UPTIME_GRACE_PERIOD_SECONDS: i64 = 300; // 5 Minutes
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
/// Required uptime percentage in a period for nodes to be eligible for rewards.
const UPTIME_SLA_PERCENT: u64 = 95;
/// The address to which the carbon credit tft will be sent.
/// Value provided by Andreas Hartl in a Telegram DM on 03-02-2022
const CARBON_CREDIT_ADDRESS: &str = "GDIJY6K2BBRIRX423ZFUYKKFDN66XP2KMSBZFQSE2PSNDZ6EDVQTRLSU";
/// Address of horizon server to use
//const HORIZON_URL: &str = "https://stellar-mainnet.grid.tf";
const HORIZON_URL: &str = "https://horizon.stellar.org";

fn main() {
    // TODO: use `clap` to properly have flags for this
    let network = Network::Test;
    let mut args = std::env::args();
    // ignore binary name
    args.next();

    let period_offset = args.next().unwrap().parse().unwrap();
    let period = Period::at_offset(period_offset);
    let start_ts: i64 = period.start();
    let end_ts: i64 = period.end();
    let wss_url = args.next().unwrap();
    // Dump state maybe?
    let stop_block = args.next().map(|s| s.parse::<u32>().unwrap());

    // load previous receipts
    let previous_period_offset = period_offset - 1;
    println!("Loading receipts from period {}", previous_period_offset);
    let mut previous_receipt_dir = path::PathBuf::new();
    previous_receipt_dir.push("receipts");
    previous_receipt_dir.push(previous_period_offset.to_string());

    let mut previous_receipts = HashMap::new();

    match std::fs::read_dir(&previous_receipt_dir) {
        Err(_) => println!("Previous receipt dir does not exist, skip loading receipts"),
        Ok(dir_iter) => {
            for file in dir_iter {
                let file = file.unwrap();
                if !file.file_type().unwrap().is_file() {
                    continue;
                }
                let mut hash = [0; 32];
                hex::decode_to_slice(file.file_name().as_bytes(), &mut hash).unwrap();
                let mut path = previous_receipt_dir.clone();
                path.push(file.file_name());
                let data = fs::read_to_string(path).unwrap();
                let receipt: MintingReceipt = serde_json::from_str(&data).unwrap();
                previous_receipts.insert(hash, receipt);
            }
        }
    }

    // load previous fixup receipts
    println!(
        "Loading fixup receipts from period {}",
        previous_period_offset
    );
    let mut previous_fixup_receipt_dir = path::PathBuf::new();
    previous_fixup_receipt_dir.push("receipts");
    previous_fixup_receipt_dir.push("fixed");
    previous_fixup_receipt_dir.push(previous_period_offset.to_string());

    let mut previous_fixup_receipts = HashMap::new();

    match std::fs::read_dir(&previous_fixup_receipt_dir) {
        Err(_) => {
            println!("Previous fixup receipt dir does not exist, skip loading fixup receipts")
        }
        Ok(dir_iter) => {
            for file in dir_iter {
                let file = file.unwrap();
                if !file.file_type().unwrap().is_file() {
                    continue;
                }
                let mut hash = [0; 32];
                hex::decode_to_slice(file.file_name().as_bytes(), &mut hash).unwrap();
                let mut path = previous_fixup_receipt_dir.clone();
                path.push(file.file_name());
                let data = fs::read_to_string(path).unwrap();
                let receipt: FixupReceipt = serde_json::from_str(&data).unwrap();
                previous_fixup_receipts.insert(hash, receipt);
            }
        }
    }

    if previous_receipts.len() > 0 || previous_fixup_receipts.len() > 0 {
        println!(
            "Filtering receipts, pre filter length {}",
            previous_receipts.len() + previous_fixup_receipts.len()
        );
        let horizon = stellar::Horizon::new(HORIZON_URL);
        horizon.filter_previous_mints(&mut previous_receipts, &mut previous_fixup_receipts);
        println!(
            "Done filtering receipts, post filter length {}",
            previous_receipts.len() + previous_fixup_receipts.len()
        );
    }

    let client =
        SharedClient::<sp_core::sr25519::Pair, tfchain_client::runtimes::runtime::Event>::new(
            Client::new(wss_url, None),
        );

    println!("Finding start block");
    let start_block = client.height_at_timestamp(start_ts).unwrap();
    println!("Finding end block");
    let mut end_block = client.height_at_timestamp(end_ts).unwrap();
    if let Some(stop_block) = stop_block {
        println!("Overriding end block to {}", stop_block);
        end_block = stop_block;
    }

    // Grab existing nodes
    let mut nodes: BTreeMap<_, _> = Window::at_height(client.clone(), start_block, network)
        .unwrap()
        .unwrap()
        .nodes()
        .unwrap()
        .into_iter()
        .map(|node| {
            let node = node.unwrap();
            (
                node.id,
                MintingNode {
                    id: node.id,
                    farm_id: node.farm_id,
                    twin_id: node.twin_id,
                    resources: unsafe { mem::transmute(node.resources) },
                    location: unsafe { mem::transmute(node.location) },
                    country: node.country,
                    city: node.city,
                    created: node.created,
                    certification_type: unsafe { mem::transmute(node.certification) },
                    uptime_info: None,
                    first_uptime_violation: None,
                    connected: NodeConnected::Old,
                    connection_price: node.connection_price,
                    capacity_consumption: TotalConsumption::default(),
                    virtualized: node.virtualized,
                    farming_policy_id: node.farming_policy_id,
                },
            )
        })
        .collect();
    println!("Found {} existing nodes", nodes.len());

    // Load farms at the end of the period. This means we don't have to parse individual farm
    // events, as we can just fetch the last known state.
    let farm_window = Window::at_height(client.clone(), end_block, network)
        .unwrap()
        .unwrap();

    let farms: BTreeMap<_, _> = farm_window
        .farms()
        .unwrap()
        .into_iter()
        .map(|farm| {
            let farm = farm.unwrap();
            (farm.id, farm)
        })
        .collect();

    let payout_addresses: BTreeMap<_, _> = farms
        .iter()
        .map(|(id, _)| (id, farm_window.farm_payout_address(*id).unwrap()))
        .filter_map(|(id, address)| match address {
            Some(address) => Some((*id, address)),
            None => None,
        })
        .collect();

    // Grab existing contracts
    let mut contracts: BTreeMap<_, _> = Window::at_height(client.clone(), start_block, network)
        .unwrap()
        .unwrap()
        .contracts(false)
        .unwrap()
        .into_iter()
        .filter_map(|contract| {
            let (contract, resources) = contract.unwrap();
            // Namecontract is actually billed once deployed through a node contract.
            if let ContractData::NodeContract(nc) =
                unsafe { mem::transmute(contract.contract_type) }
            {
                Some((
                    contract.contract_id,
                    Contract {
                        contract_id: contract.contract_id,
                        node_id: nc.node_id,
                        // a report should pop up for this
                        last_report_ts: 0,
                        ips: nc.public_ips,
                        resources: unsafe { mem::transmute(resources) },
                    },
                ))
            } else {
                None
            }
        })
        .collect();
    println!("Found {} existing contracts", contracts.len());

    // Get farming policies
    let farming_policies: BTreeMap<_, _> = Window::at_height(client.clone(), end_block, network)
        .unwrap()
        .unwrap()
        .farm_policies()
        .unwrap()
        .into_iter()
        .filter_map(|policy| {
            let policy = policy.unwrap();
            Some((policy.id, policy))
        })
        .collect();
    println!("Found {} farming policies", farming_policies.len());

    println!("Setup block import pipeline");
    let blocks = end_block - start_block + 1;
    let block_stream = block_import(client.clone(), start_block, end_block, network);

    let bar = ProgressBar::new(blocks as u64);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("[Time on chain: {msg}] {wide_bar} {pos:>6}/{len:>6} (ETA: {eta_precise})"),
    );
    let mut last_height = start_block - 1;
    for block in block_stream {
        assert_eq!(block.height, last_height + 1);

        for event in block.events {
            match event {
                TfchainEvent::TFGrid(grid_event) => match *grid_event {
                    TFGridEvent::NodeStored(node) => {
                        nodes.insert(
                            node.id,
                            MintingNode {
                                id: node.id,
                                farm_id: node.farm_id,
                                twin_id: node.twin_id,
                                resources: unsafe { mem::transmute(node.resources) },
                                location: unsafe { mem::transmute(node.location) },
                                country: node.country,
                                city: node.city,
                                created: node.created,
                                certification_type: unsafe { mem::transmute(node.certification) },
                                uptime_info: None,
                                first_uptime_violation: None,
                                connected: NodeConnected::Current(block.timestamp.timestamp()),
                                connection_price: node.connection_price,
                                capacity_consumption: TotalConsumption::default(),
                                virtualized: node.virtualized,
                                farming_policy_id: node.farming_policy_id,
                            },
                        );
                    }
                    TFGridEvent::NodeUpdated(node) => {
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
                        old_node.location = unsafe { mem::transmute(node.location) };
                        old_node.resources = unsafe { mem::transmute(node.resources) };
                        old_node.country = node.country;
                        old_node.city = node.city;
                        // Don't care about "create" as that should be fixed anyway
                        // Update certification type. It's technically possible for a node to jump
                        // from DIY to certified and back in the same period, but practically that
                        // should not happen.
                        old_node.certification_type = unsafe { mem::transmute(node.certification) };
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
                    }
                    TFGridEvent::NodeUptimeReported(id, current_time, reported_uptime) => {
                        // Sanity check the event, this should not be needed
                        assert_eq!(current_time as i64, block.timestamp.timestamp());
                        let node = match nodes.get_mut(&id) {
                            Some(node) => node,
                            None => panic!(
                                "can't report uptime for unknown node {} in block {}",
                                id, block.height
                            ),
                        };
                        if let Some((last_reported_at, last_reported_uptime, mut total_uptime)) =
                            node.uptime_info
                        {
                            let report_delta = current_time as i64 - last_reported_at;
                            let uptime_delta = reported_uptime as i64 - last_reported_uptime as i64;
                            // There are quite some situations here. Notice that due to the
                            // blockchain only producing blocks every 6 seconds, and network delay
                            // + a host of other issues, we will allow a node to report uptime with
                            // "grace period" of a couple of minutes or so in either direction.
                            //
                            // 1. uptime_delta > report_delta + GRACE_PERIOD. Node is talking
                            //    rubish.
                            if uptime_delta > report_delta + UPTIME_GRACE_PERIOD_SECONDS {
                                // This is possible if a node lags when sending uptime (extrinsic
                                // takes a while to be accepted). Manual data validation found no
                                // issues (i.e. all incidents of this type were a result of the
                                // above). This should be changed in the future.
                                total_uptime += uptime_delta as u64;
                                node.uptime_info =
                                    Some((current_time as i64, reported_uptime, total_uptime));
                                continue;
                            }
                            // 2. The difference in uptime is within reason of the difference in
                            //    report times, i.e. the node is properly reporting.
                            if uptime_delta <= report_delta + UPTIME_GRACE_PERIOD_SECONDS
                                && uptime_delta >= report_delta - UPTIME_GRACE_PERIOD_SECONDS
                            {
                                // It is technically possible for the delta to be less than 0 and
                                // within the expected time frame. If nodes boot, send uptime, then
                                // immediately reboot that is possible. In those cases, handle that
                                // below, as that is the reboot detection.
                                if uptime_delta > 0 {
                                    // Simply add the uptime delta. If this is too large or low by a
                                    // couple of seconds it will be corrected by the next pings anyhow.
                                    total_uptime += uptime_delta as u64;
                                    node.uptime_info =
                                        Some((current_time as i64, reported_uptime, total_uptime));
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
                                total_uptime += reported_uptime;
                                node.uptime_info =
                                    Some((current_time as i64, reported_uptime, total_uptime));
                                continue;
                            }
                            //    2. Uptime is higher than previously recorded uptime but too low.
                            //    This might be a result off network congestion.
                            if reported_uptime > last_reported_uptime {
                                total_uptime += uptime_delta as u64;
                                node.uptime_info =
                                    Some((current_time as i64, reported_uptime, total_uptime));
                                continue;
                            }
                            //    3. Uptime is too high, this is garbage
                            if node.first_uptime_violation.is_none() {
                                node.first_uptime_violation =
                                    Some((last_reported_at, block.height));
                            }
                            continue;
                        } else {
                            let period_duration = current_time as i64 - start_ts;
                            // Make sure we don't give more credit than the current length of the
                            // period.
                            let up_in_period =
                                std::cmp::min(period_duration as u64, reported_uptime);
                            // Save uptime info
                            node.uptime_info =
                                Some((current_time as i64, reported_uptime, up_in_period));
                        }
                    }
                    _ => {}
                },
                TfchainEvent::SmartContract(contract_event) => match contract_event {
                    SmartContractEvent::UpdatedUsedResources(contract_id, resources) => {
                        let contract = match contracts.get_mut(&contract_id) {
                            Some(contract) => contract,
                            // Contract needs to exist.
                            None => {
                                panic!("Can't set used resources for contract {} which does not exist in block {}", contract_id, block.height);
                            }
                        };
                        contract.resources = unsafe { mem::transmute(resources) };
                    }
                    SmartContractEvent::NruConsumption(contract_id, timestamp, window, nru) => {
                        let contract = match contracts.get_mut(&contract_id) {
                            Some(contract) => contract,
                            // Contract needs to exist.
                            None => {
                                panic!("Can't set used resources for contract {} which does not exist in block {}", contract_id, block.height);
                            }
                        };
                        let node = match nodes.get_mut(&contract.node_id) {
                            Some(node) => node,
                            None => {
                                panic!(
                                    "can't process consumption for unknown node {} in block {}",
                                    contract.node_id, block.height
                                )
                            }
                        };
                        // Just to make sure reports are ordered
                        if timestamp as i64 <= contract.last_report_ts {
                            // Silently ignore reports out of order, we already covered this in an
                            // already processed consumption report. This can happen if the node pushes
                            // a contract consumption report twice.
                            continue;
                        }

                        // If report ts predates start we ignore it.
                        if (timestamp as i64) < start_ts {
                            continue;
                        }
                        node.capacity_consumption.cru += (contract.resources.cru * window) as u128;
                        node.capacity_consumption.mru += (contract.resources.mru * window) as u128;
                        node.capacity_consumption.hru += (contract.resources.hru * window) as u128;
                        node.capacity_consumption.sru += (contract.resources.sru * window) as u128;
                        node.capacity_consumption.ips += contract.ips as u64 * window;
                        node.capacity_consumption.nru += nru;
                        contract.last_report_ts = timestamp as i64;
                    }
                    SmartContractEvent::ContractCreated(contract) => {
                        // we only care about node contracts
                        if let ContractData::NodeContract(nc) =
                            unsafe { mem::transmute(contract.contract_type) }
                        {
                            contracts.insert(
                                contract.contract_id,
                                Contract {
                                    contract_id: contract.contract_id,
                                    node_id: nc.node_id,
                                    last_report_ts: block.timestamp.timestamp(),
                                    ips: nc.public_ips,
                                    resources: Resources::default(),
                                },
                            );
                        };
                    }
                    SmartContractEvent::NodeContractCanceled(_, _, _) => {
                        // We can't cancel the contract, as it might still receive a consumption
                        // report. Technically we should check that only 1 final report is
                        // received. But honestly it's the chain's job to make sure of that. Now,
                        // considering IP, we will not do special handling, instead we rely on the
                        // fact that IP reservations are only credited once a consumption report is
                        // processed.
                    }
                    _ => {}
                },
                _ => {}
            };
        }

        // update last height for sanity check
        last_height = block.height;
        // finally update progress bar
        bar.set_message(block.timestamp.to_rfc2822());
        bar.inc(1);
    }

    bar.finish();

    let encoded_nodes = bincode::serialize(&nodes).unwrap();
    let encoded_farms = bincode::serialize(&farms).unwrap();
    let encoded_contracts = bincode::serialize(&contracts).unwrap();
    let encoded_payout_addresses = bincode::serialize(&payout_addresses).unwrap();
    let encoded_farming_policies = bincode::serialize(&farming_policies).unwrap();

    std::fs::File::create("nodes.bin")
        .unwrap()
        .write_all(&encoded_nodes)
        .unwrap();
    std::fs::File::create("farms.bin")
        .unwrap()
        .write_all(&encoded_farms)
        .unwrap();
    std::fs::File::create("payout_addresses.bin")
        .unwrap()
        .write_all(&encoded_payout_addresses)
        .unwrap();
    std::fs::File::create("contracts.bin")
        .unwrap()
        .write_all(&encoded_contracts)
        .unwrap();
    std::fs::File::create("farming_policies.bin")
        .unwrap()
        .write_all(&encoded_farming_policies)
        .unwrap();
    // DONE here
    return;

    // println!("Getting uptime info from post period");
    // let block_stream = block_import(client, end_block, end_block + BLOCKS_IN_HOUR * 2, network);
    // let bar = ProgressBar::new(BLOCKS_IN_HOUR as u64 * 2);
    // bar.set_style(
    //     ProgressStyle::default_bar()
    //         .template("[Time on chain: {msg}] {wide_bar} {pos:>6}/{len:>6} (ETA: {eta_precise})"),
    // );

    // // Collect post-period uptime events. Violations don't matter here, those will be handled next
    // // period.
    // for block in block_stream {
    //     for event in block.events {
    //         if let TfchainEvent::TFGrid(event) = event {
    //             if let TFGridEvent::NodeUptimeReported(id, current_time, reported_uptime) = *event {
    //                 // Sanity check the event, this should not be needed
    //                 assert_eq!(current_time as i64, block.timestamp.timestamp());
    //                 let node = match nodes.get_mut(&id) {
    //                     Some(node) => node,
    //                     // This is possible if its an uptime report for a node which came online after
    //                     // the period ended
    //                     None => continue,
    //                 };
    //                 if let Some((last_reported_at, last_reported_uptime, mut total_uptime)) =
    //                     node.uptime_info
    //                 {
    //                     // only collect 1 uptime event after the period ended
    //                     if last_reported_at >= end_ts {
    //                         continue;
    //                     }
    //                     let report_delta = current_time as i64 - last_reported_at;
    //                     let uptime_delta = reported_uptime as i64 - last_reported_uptime as i64;
    //                     let delta_in_period = end_ts - last_reported_at;
    //                     // There are quite some situations here. Notice that due to the
    //                     // blockchain only producing blocks every 6 seconds, and network delay
    //                     // + a host of other issues, we will allow a node to report uptime with
    //                     // "grace period" of a minute or so in either direction.
    //                     //
    //                     // 1. uptime_delta > report_delta + GRACE_PERIOD. Node is talking
    //                     //    rubish.
    //                     if uptime_delta > report_delta + UPTIME_GRACE_PERIOD_SECONDS {
    //                         // This will actually be picked up next period as a violation if we
    //                         // care for that.
    //                         total_uptime += delta_in_period as u64;
    //                         node.uptime_info =
    //                             Some((current_time as i64, reported_uptime, total_uptime));
    //                         continue;
    //                     }
    //                     // 2. The difference in uptime is within reason of the difference in
    //                     //    report times, i.e. the node is properly reporting.
    //                     if uptime_delta <= report_delta + UPTIME_GRACE_PERIOD_SECONDS
    //                         && uptime_delta >= report_delta - UPTIME_GRACE_PERIOD_SECONDS
    //                     {
    //                         // It is technically possible for the delta to be less than 0 and
    //                         // within the expected time frame. If nodes boot, send uptime, then
    //                         // immediately reboot that is possible. In those cases, handle that
    //                         // below, as that is the reboot detection.
    //                         if uptime_delta > 0 {
    //                             // Simply add the uptime delta. If this is too large or low by a
    //                             // couple of seconds it will be corrected by the next pings anyhow.
    //                             //
    //                             // Make sure we don't add too much based on the period.
    //                             total_uptime += delta_in_period as u64;
    //                             node.uptime_info =
    //                                 Some((current_time as i64, reported_uptime, total_uptime));
    //                             continue;
    //                         }
    //                     }
    //                     // 3. The difference in uptime is too low. Again there are multiple
    //                     //    scenarios. Either way we consider the node rebooted. Depending on
    //                     //    the reported uptime, the node reports legit uptime, or it reports
    //                     //    an uptime which is too high.
    //                     //
    //                     //    1. Uptime is within bounds.
    //                     if reported_uptime as i64 <= report_delta {
    //                         // Account for the fact that we are actually out of the period
    //                         let out_of_period = current_time - end_ts as u64;
    //                         if out_of_period < reported_uptime {
    //                             total_uptime += reported_uptime - out_of_period;
    //                         }
    //                         node.uptime_info =
    //                             Some((current_time as i64, reported_uptime, total_uptime));
    //                         continue;
    //                     }
    //                     //    2. Uptime is higher than previously recorded uptime but too low.
    //                     //    This might be a result off network congestion.
    //                     if reported_uptime > last_reported_uptime {
    //                         total_uptime += uptime_delta as u64;
    //                         node.uptime_info =
    //                             Some((current_time as i64, reported_uptime, total_uptime));
    //                         continue;
    //                     }
    //                     //    3. Uptime is too high, this is garbage
    //                     if node.first_uptime_violation.is_none() {
    //                         node.first_uptime_violation = Some((last_reported_at, block.height));
    //                         continue;
    //                     }

    //                     // We should have handled all cases. Make this explicit here.
    //                     unreachable!();
    //                 }
    //             }
    //         }
    //     }

    //     bar.set_message(block.timestamp.to_rfc2822());
    //     bar.inc(1)
    // }
    // bar.finish_and_clear();

    // let mut receipts = BTreeMap::new();
    // let mut payout_file = std::fs::File::create("payouts.csv").unwrap();
    // let mut overview_file = std::fs::File::create("overview.csv").unwrap();
    // let mut retry_file = std::fs::File::create("retries.csv").unwrap();

    // let mut carbon_tft_units = 0;

    // writeln!(overview_file,"node id,twin id,farm name (farm id),period start,period end,measured uptime,CU,SU,NU,USD reward,TFT reward,TFT price on connect,carbon offset USD generated,carbon offset TFT generated,cru,cru used,mru,mru used,hru,hru used,sru,sru used,IP used,DIY state,Virtualized,violation,stellar address").unwrap();
    // for (_, node) in nodes {
    //     let node_period = node.real_period(period);
    //     let node_period_duration = node_period.duration();
    //     let node_start = Utc.timestamp(node_period.start(), 0);
    //     let node_end = Utc.timestamp(node_period.end(), 0);
    //     let (cu, su, nu) = node.cloud_units_permill();
    //     let (musd, tft) = node.scaled_payout(period, &farming_policies);
    //     let (co_musd, co_tft) = node.scaled_carbon_payout(period);
    //     let cru_used = (node.capacity_consumption.cru / node_period_duration as u128) as u64;
    //     let mru_used = (node.capacity_consumption.mru / node_period_duration as u128) as u64;
    //     let hru_used = (node.capacity_consumption.hru / node_period_duration as u128) as u64;
    //     let sru_used = (node.capacity_consumption.sru / node_period_duration as u128) as u64;
    //     let farm = if let Some(farm) = farms.get(&node.farm_id) {
    //         farm
    //     } else {
    //         println!(
    //             "node {} is in farm {} which does not exist anymore",
    //             node.id, node.farm_id
    //         );
    //         continue;
    //     };
    //     let stellar_address = if let Some(stellar_address) = payout_addresses.get(&node.farm_id) {
    //         &stellar_address
    //     } else {
    //         ""
    //     };
    //     writeln!(overview_file,
    //         "{},{},{} ({}),{},{},{},{},{},{},{} $,{},{} $,{} $,{} TFT,{},{:.2}%,{},{:.2}%,{},{:.2}%,{},{:.2}%,{:.2} hours,{},{},{},{}",
    //         node.id,
    //         node.twin_id,
    //         farm.name,
    //         node.farm_id,
    //         node_start,
    //         node_end,
    //         node.uptime(period),
    //         format_args!("{}.{:06}", cu / ONE_MILL as u64, cu % ONE_MILL as u64),
    //         format_args!("{}.{:06}", su / ONE_MILL as u64, su % ONE_MILL as u64),
    //         format_args!("{}.{:06}", nu / ONE_MILL as u64, nu % ONE_MILL as u64),
    //         format_args!("{}.{:03}", musd / 1_000, musd % 1_000),
    //         format_args!("{}.{:07}", tft / UNITS_PER_TFT, tft % UNITS_PER_TFT),
    //         format_args!(
    //             "{}.{:03}",
    //             node.connection_price / 1_000,
    //             node.connection_price % 1_000
    //         ),
    //         format_args!("{}.{:03}", co_musd / 1_000, co_musd % 1_000),
    //         format_args!("{}.{:07}", co_tft / UNITS_PER_TFT, co_tft % UNITS_PER_TFT),
    //         node.resources.cru,
    //         if node.resources.cru > 0 {cru_used as f64 * 100. / node.resources.cru as f64} else { 0.},
    //         node.resources.mru,
    //         if node.resources.mru > 0 {mru_used as f64 * 100. / node.resources.mru as f64} else {0.},
    //         node.resources.hru,
    //         if node.resources.hru > 0 {hru_used as f64 * 100. / node.resources.hru as f64} else {0.},
    //         node.resources.sru,
    //         if node.resources.sru > 0 {sru_used as f64 * 100. / node.resources.sru as f64} else {0.},
    //         node.capacity_consumption.ips as f64 / 3600.,
    //         if let NodeCertification::Certified = node.certification_type {
    //             "CERTIFIED"
    //         } else {
    //             "DIY"
    //         },
    //         node.virtualized,
    //         if let Some((lra, violation)) = node.first_uptime_violation {
    //             format!("violation of uptime measurement in block {} (previous report {})", violation, lra)
    //         } else {
    //             "".into()
    //         },
    //         stellar_address,
    //     ).unwrap();

    //     let receipt = node.receipt(period, &farms, &payout_addresses, &farming_policies);
    //     if !stellar_address.is_empty() && tft != 0 {
    //         writeln!(
    //             payout_file,
    //             "{},{}.{:07},{}",
    //             stellar_address,
    //             tft / UNITS_PER_TFT,
    //             tft % UNITS_PER_TFT,
    //             hex::encode(receipt.hash()),
    //         )
    //         .unwrap();
    //     }
    //     receipts.insert(receipt.hash(), receipt);

    //     // Count carbon TFT credits
    //     carbon_tft_units += co_tft;
    // }

    // // Retry payments once
    // let mut retry_receipts = HashMap::new();
    // for (hash, failed_receipt) in previous_receipts {
    //     // no point in doing this
    //     if failed_receipt.reward.tft == 0 {
    //         continue;
    //     }
    //     let retry_receipt = RetryPayoutReceipt {
    //         failed_payout_period: failed_receipt.period,
    //         retry_period: period,
    //         farm_id: failed_receipt.farm_id,
    //         previous_stellar_payout_address: failed_receipt.stellar_payout_address,
    //         stellar_payout_address: payout_addresses
    //             .get(&failed_receipt.farm_id)
    //             .map(|a| a.clone())
    //             .unwrap_or("".to_string()),
    //         retry_for_receipt: hex::encode(hash),
    //         reward: failed_receipt.reward,
    //     };
    //     let retry_hash = hex::encode(retry_receipt.hash());

    //     if !retry_receipt.stellar_payout_address.is_empty() && retry_receipt.reward.tft != 0 {
    //         writeln!(
    //             payout_file,
    //             "{},{}.{:07},{}",
    //             retry_receipt.stellar_payout_address,
    //             retry_receipt.reward.tft / UNITS_PER_TFT,
    //             retry_receipt.reward.tft % UNITS_PER_TFT,
    //             retry_hash,
    //         )
    //         .unwrap();
    //     }

    //     retry_receipts.insert(retry_hash, retry_receipt);
    // }

    // let mut retry_fixed_receipts = HashMap::new();
    // for (hash, failed_receipt) in previous_fixup_receipts {
    //     // no point in doing this
    //     if failed_receipt.fixup_reward.tft == 0 {
    //         continue;
    //     }
    //     let retry_receipt = RetryPayoutReceipt {
    //         failed_payout_period: failed_receipt.period,
    //         retry_period: period,
    //         farm_id: failed_receipt.farm_id,
    //         previous_stellar_payout_address: failed_receipt.stellar_payout_address,
    //         stellar_payout_address: payout_addresses
    //             .get(&failed_receipt.farm_id)
    //             .map(|a| a.clone())
    //             .unwrap_or("".to_string()),
    //         retry_for_receipt: hex::encode(hash),
    //         reward: failed_receipt.fixup_reward,
    //     };
    //     let retry_hash = hex::encode(retry_receipt.hash());

    //     if !retry_receipt.stellar_payout_address.is_empty() && retry_receipt.reward.tft != 0 {
    //         writeln!(
    //             payout_file,
    //             "{},{}.{:07},{}",
    //             retry_receipt.stellar_payout_address,
    //             retry_receipt.reward.tft / UNITS_PER_TFT,
    //             retry_receipt.reward.tft % UNITS_PER_TFT,
    //             retry_hash,
    //         )
    //         .unwrap();
    //     }

    //     retry_fixed_receipts.insert(retry_hash, retry_receipt);
    // }

    // // Sort hashes in lexicographical order
    // let mut receipt_hashes = receipts.keys().cloned().collect::<Vec<_>>();
    // receipt_hashes.sort_unstable();
    // let mut hasher = Blake2b::<U32>::new();
    // for receipt_hash in receipt_hashes {
    //     hasher.update(receipt_hash);
    // }
    // let carbon_hash: [u8; 32] = hasher.finalize().into();
    // writeln!(
    //     payout_file,
    //     "{},{}.{:07},{}",
    //     CARBON_CREDIT_ADDRESS,
    //     carbon_tft_units / UNITS_PER_TFT,
    //     carbon_tft_units % UNITS_PER_TFT,
    //     hex::encode(carbon_hash),
    // )
    // .unwrap();

    // // Write generated receipts
    // let mut receipt_dir = path::PathBuf::new();
    // receipt_dir.push("receipts");
    // receipt_dir.push(period_offset.to_string());
    // std::fs::create_dir_all(&receipt_dir).unwrap();
    // for (hash, receipt) in receipts {
    //     let mut path = receipt_dir.clone();
    //     path.push(hex::encode(hash));
    //     std::fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    // }

    // // Write retry receipts
    // writeln!(
    //     retry_file,
    //     "farm_id,previous_stellar_address,new_stellar_address,amount TFT,retry_for",
    // )
    // .unwrap();

    // let mut retry_receipt_dir = path::PathBuf::new();
    // retry_receipt_dir.push("receipts");
    // retry_receipt_dir.push("retries");
    // retry_receipt_dir.push(period_offset.to_string());
    // std::fs::create_dir_all(&retry_receipt_dir).unwrap();
    // for (hash, receipt) in retry_receipts {
    //     writeln!(
    //         retry_file,
    //         "{},{},{},{}.{:07},{}",
    //         receipt.farm_id,
    //         receipt.previous_stellar_payout_address,
    //         receipt.stellar_payout_address,
    //         receipt.reward.tft / UNITS_PER_TFT,
    //         receipt.reward.tft % UNITS_PER_TFT,
    //         receipt.retry_for_receipt,
    //     )
    //     .unwrap();
    //     let mut path = retry_receipt_dir.clone();
    //     path.push(hash);
    //     std::fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    // }
    // for (hash, receipt) in retry_fixed_receipts {
    //     writeln!(
    //         retry_file,
    //         "{},{},{},{}.{:07},{}",
    //         receipt.farm_id,
    //         receipt.previous_stellar_payout_address,
    //         receipt.stellar_payout_address,
    //         receipt.reward.tft / UNITS_PER_TFT,
    //         receipt.reward.tft % UNITS_PER_TFT,
    //         receipt.retry_for_receipt,
    //     )
    //     .unwrap();
    //     let mut path = retry_receipt_dir.clone();
    //     path.push(hash);
    //     std::fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    // }
}

fn block_import<P, E>(
    client: SharedClient<P, E>,
    start: BlockNumber,
    end: BlockNumber,
    network: tfchain_client::window::Network,
) -> mpsc::Receiver<MintingBlock>
where
    P: Pair,
    MultiSignature: From<P::Signature>,
    E: tfchain_client::support::sp_runtime::traits::Member + tfchain_client::support::Parameter,
    TfchainEvent: From<E>,
    tfchain_client::window::EventTypedClient<P>: From<SharedClient<P, E>>,
{
    let mut receivers = Vec::with_capacity(RPC_THREADS);

    for i in 0..RPC_THREADS {
        let (tx, rx) = mpsc::sync_channel(PRE_FETCH);
        receivers.push(rx);
        let client = client.clone();
        let mut height = start + i as u32;
        std::thread::spawn(move || loop {
            if height > end {
                break;
            }
            let window = match Window::at_height(client.clone(), height, network) {
                Ok(maybe_window) => maybe_window.unwrap(),
                Err(e) => {
                    eprintln!("Could not create window at height {}: {}", height, e);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
            };
            let timestamp = match window.date() {
                Ok(ts) => ts,
                Err(e) => {
                    eprintln!("Could not fetch time at height {}: {}", height, e);
                    continue;
                }
            };
            let events = match window.events() {
                Ok(evt) => evt,
                Err(e) => {
                    eprintln!("Failed to fetch events at height {}: {}", height, e);
                    continue;
                }
            };
            tx.send(MintingBlock {
                height,
                timestamp,
                events,
            })
            .unwrap();
            height += RPC_THREADS as u32;
        });
    }

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || loop {
        let mut disconnects = 0;
        for rx in &receivers {
            let res = match rx.recv() {
                Ok(res) => res,
                Err(_) => {
                    disconnects += 1;
                    continue;
                }
            };
            tx.send(res).unwrap();
        }
        if disconnects == receivers.len() {
            break;
        }
    });

    rx
}

struct MintingBlock {
    height: BlockNumber,
    timestamp: DateTime<Utc>,
    events: Vec<TfchainEvent>,
}

#[derive(Serialize, Deserialize)]
enum NodeConnected {
    /// Node was connected in a previous period.
    Old,
    /// Node was connected in the current period, first seen at the specified timestamp.
    Current(i64),
}

#[derive(Serialize, Deserialize)]
struct MintingNode {
    id: u32,
    farm_id: u32,
    twin_id: u32,
    resources: Resources,
    location: Location,
    country: String,
    city: String,
    created: u64,
    certification_type: NodeCertification,
    // (last ping, last reported uptime, total uptime).
    uptime_info: Option<(i64, u64, u64)>,
    // Block where the first uptime report violation was detected, if any.
    first_uptime_violation: Option<(i64, u32)>,
    connected: NodeConnected,
    // TFT price expressed in USD at time of connection. Price is expressed in mUSD (3 digits
    // precision). I.e. 1 USD => 1000.
    connection_price: u32,
    // capacity consumed by workloads over a period.
    capacity_consumption: TotalConsumption,
    virtualized: bool,
    farming_policy_id: u32,
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
    fn node_payout_musd(
        &self,
        farming_policies: &BTreeMap<u32, FarmingPolicy<BlockNumber>>,
    ) -> u64 {
        if self.virtualized || self.first_uptime_violation.is_some() {
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
    fn node_payout_tft_units(
        &self,
        farming_policies: &BTreeMap<u32, FarmingPolicy<BlockNumber>>,
    ) -> u64 {
        // connection price is in mUSD.
        // TODO: revert once fixed on chain (presumably period 54)
        // self.node_payout_musd(farming_policies) * UNITS_PER_TFT / self.connection_price as u64
        self.node_payout_musd(farming_policies) * UNITS_PER_TFT / 80
    }

    /// Calculate the amount of mUSD generated by the node for carbon offset.
    ///
    /// This is solely based on the CU and SU, therefore it can be calculated from just the node
    /// definition without aggregating the blocks of a period.
    ///
    /// A virtualized node is garbage so that does not receive anything.
    fn node_carbon_musd(&self) -> u64 {
        if self.virtualized || self.first_uptime_violation.is_some() {
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
        farming_policies: &BTreeMap<u32, FarmingPolicy<BlockNumber>>,
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

    /// Indicates if a node managed to achieve it's SLA in a period.
    ///
    /// Currently SLA is the same for all nodes, this might change in the future.
    fn sla_achieved(&self, period: Period) -> bool {
        let scaled_period = self.real_period(period);
        self.uptime(scaled_period) * 100 / scaled_period.duration() >= UPTIME_SLA_PERCENT
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
        farming_policies: &BTreeMap<u32, FarmingPolicy<BlockNumber>>,
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
            if self.farming_policy_id == 1 {
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

#[derive(Serialize, Deserialize)]
struct Contract {
    contract_id: u64,
    node_id: u32,
    // timestamp of last report. If not set, time when the contract was created.
    last_report_ts: i64,
    ips: u32,
    // Resources set on chain
    resources: Resources,
}

#[derive(Default, Serialize, Deserialize)]
struct TotalConsumption {
    // cru mru hru sru and ips is value * time i.e. unit seconds
    cru: u128,
    sru: u128,
    hru: u128,
    mru: u128,
    ips: u64,
    nru: u64,
}
