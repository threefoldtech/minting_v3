use chrono::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::{collections::BTreeMap, sync::mpsc};
use tfchain_client::{
    client::{Client, MultiSignature, Pair, SharedClient},
    events::{SmartContractEvent, TFGridEvent, TfchainEvent},
    types::{BlockNumber, CertificationType, ContractData, Location, Resources},
    window::Window,
};

const RPC_THREADS: usize = 100;
const PRE_FETCH: usize = 5;
const UPTIME_GRACE_PERIOD_SECONDS: i64 = 60;
const GIB: u128 = 1024 * 1024 * 1024;
const ONE_MILL: u128 = 1_000_000;
/// Reward for 1 CU per period in mUSD.
/// Value taken from [the
/// wiki](https://library.threefold.me/info/threefold#/tfgrid/farming/threefold__farming_reward?id=farming-reward-calculation)
/// on 31-01-2022.
const CU_REWARD_MUSD: u64 = 2400;
/// Reward for 1 SU per period in mUSD.
/// Value taken from [the
/// wiki](https://library.threefold.me/info/threefold#/tfgrid/farming/threefold__farming_reward?id=farming-reward-calculation)
/// on 31-01-2022.
const SU_REWARD_MUSD: u64 = 1000;
/// Reward for 1 NU per period in mUSD.
/// Value taken from [the
/// wiki](https://library.threefold.me/info/threefold#/tfgrid/farming/threefold__farming_reward?id=farming-reward-calculation)
/// on 31-01-2022.
const NU_REWARD_MUSD: u64 = 30;
/// Reward for 1 IP reserved for 1 hour in mUSD.
/// Value taken from [the
/// wiki](https://library.threefold.me/info/threefold#/tfgrid/farming/threefold__farming_reward?id=farming-reward-calculation)
/// on 31-01-2022.
const IP_REWARD_MUSD: u64 = 5;
/// Price of TFT in mUSD.
/// Value taken from [the
/// wiki](https://library.threefold.me/info/threefold#/tfgrid/farming/threefold__farming_reward?id=farming-reward-calculation)
/// on 31-01-2022.
const DAO_CONNECTION_PRICE_MUSD: u64 = 80; // 0.08USD
/// The amount of "units" that make 1 TFT.
const UNITS_PER_TFT: u64 = 10_000_000;
/// The amount of blocks expected in an hour.
const BLOCKS_IN_HOUR: u32 = 10 * 60; // 10 blocks per minute

fn main() {
    let mut args = std::env::args();
    // ignore binary name
    args.next();

    let start_ts: i64 = args.next().unwrap().parse().unwrap();
    let end_ts: i64 = args.next().unwrap().parse().unwrap();
    let wss_url = args.next().unwrap();

    let client = SharedClient::<sp_core::sr25519::Pair>::new(Client::new(wss_url, None));

    println!("Finding start block");
    let start_block = client.height_at_timestamp(start_ts).unwrap();
    println!("Finding end block");
    let end_block = client.height_at_timestamp(end_ts).unwrap();

    // Grab existing nodes
    let mut nodes: BTreeMap<_, _> = Window::at_height(client.clone(), start_block)
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
                    resources: node.resources,
                    location: node.location,
                    country: node.country,
                    city: node.city,
                    created: node.created,
                    certification_type: node.certification_type,
                    uptime_info: None,
                    first_uptime_violation: None,
                    connected: NodeConnected::Old,
                    connection_price: DAO_CONNECTION_PRICE_MUSD,
                    capacity_consumption: TotalConsumption::default(),
                },
            )
        })
        .collect();
    println!("Found {} existing nodes", nodes.len());

    let contract_search_start_block = if BLOCKS_IN_HOUR * 48 + 1 > start_block {
        1
    } else {
        start_block - 48 * BLOCKS_IN_HOUR - 1
    };
    // Grab existing contracts
    let mut contracts: BTreeMap<_, _> =
        Window::at_height(client.clone(), contract_search_start_block)
            .unwrap()
            .unwrap()
            .contracts(false)
            .unwrap()
            .into_iter()
            .filter_map(|contract| {
                let contract = contract.unwrap();
                // Namecontract is actually billed once deployed through a node contract.
                if let ContractData::NodeContract(nc) = contract.contract_type {
                    Some((
                        contract.contract_id,
                        Contract {
                            contract_id: contract.contract_id,
                            node_id: nc.node_id,
                            // a report should pop up for this
                            last_report_ts: 0,
                            last_reported_nru: 0,
                            ips: nc.public_ips,
                        },
                    ))
                } else {
                    None
                }
            })
            .collect();
    println!("Found {} existing contracts", contracts.len());

    // Fetch the last known consumption reports for the active contracts so we have a baseline to
    // work from. It is possible that a bit of the network was consumed in the previous period.
    // This will be corrected by not gathering consumption reports after the period ends. So
    // essentially this small time window (max 1 hour) is covered in the next payout.
    println!("Extract consumption reports for existing contracts");
    let consumption_blocks =
        block_import(client.clone(), contract_search_start_block, start_block - 1);

    let bar = ProgressBar::new((start_block - contract_search_start_block) as u64 + 1);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {wide_bar} {pos:>3}/{len:>3} (ETA: {eta_precise})"),
    );
    for block in consumption_blocks {
        for event in block.events {
            if let TfchainEvent::SmartContract(contract_event) = event {
                match contract_event {
                    SmartContractEvent::ConsumptionReportReceived(report) => {
                        let contract = match contracts.get_mut(&report.contract_id) {
                            Some(contract) => contract,
                            None => panic!(
                                "can't report consumption for unknown contract {} in block {}",
                                report.contract_id, block.height
                            ),
                        };
                        contract.last_report_ts = report.timestamp as i64;
                        contract.last_reported_nru = report.nru;
                    }
                    SmartContractEvent::ContractCreated(contract) => {
                        // we only care about node contracts
                        if let ContractData::NodeContract(nc) = contract.contract_type {
                            contracts.insert(
                                contract.contract_id,
                                Contract {
                                    contract_id: contract.contract_id,
                                    node_id: nc.node_id,
                                    last_report_ts: block.timestamp.timestamp(),
                                    last_reported_nru: 0,
                                    ips: nc.public_ips,
                                },
                            );
                        };
                    }
                    _ => {}
                };
            };
        }
        bar.inc(1);
    }
    bar.finish_and_clear();

    // Remove contracts that did not get updated
    print!(
        "Removing contracts without known reports, pre filter length {}",
        contracts.len()
    );
    let mut contracts: BTreeMap<_, _> = contracts
        .into_iter()
        .filter(|(_, c)| c.last_report_ts != 0)
        .collect();
    println!(" post filter length {}", contracts.len());

    println!("Setup block import pipeline");
    let blocks = end_block - start_block + 1;
    let block_import = block_import(client, start_block, end_block);

    let bar = ProgressBar::new(blocks as u64);
    bar.set_style(
        ProgressStyle::default_bar()
            .template("[Time on chain: {msg}] {wide_bar} {pos:>6}/{len:>6} (ETA: {eta_precise})"),
    );
    let mut last_height = start_block - 1;
    for block in block_import {
        assert_eq!(block.height, last_height + 1);

        for event in block.events {
            match event {
                TfchainEvent::TFGrid(grid_event) => match grid_event {
                    TFGridEvent::NodeStored(node) => {
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
                                created: node.created,
                                certification_type: node.certification_type,
                                uptime_info: None,
                                first_uptime_violation: None,
                                connected: NodeConnected::Current(block.timestamp.timestamp()),
                                connection_price: DAO_CONNECTION_PRICE_MUSD,
                                capacity_consumption: TotalConsumption::default(),
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
                        // TODO: how to handle.
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
                        old_node.certification_type = node.certification_type;
                        // Even though this likely means the node is rebooted, don't mess with
                        // uptime_info. The reboot will be detected in the `NodeUptimeReported`
                        // handler.
                        // This does not change when the node was connected.
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
                            // "grace period" of a minute or so in either direction.
                            //
                            // 1. uptime_delta > report_delta + GRACE_PERIOD. Node is talking
                            //    rubish.
                            if uptime_delta > report_delta + UPTIME_GRACE_PERIOD_SECONDS {
                                // If we already have an uptime violation we don't care anymore.
                                if node.first_uptime_violation.is_none() {
                                    node.first_uptime_violation = Some(block.height);
                                }
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
                            //    2. Uptime is too high, this is garbage
                            if node.first_uptime_violation.is_none() {
                                node.first_uptime_violation = Some(block.height);
                                continue;
                            }

                            // We should have handled all cases. Make this explicit here.
                            unreachable!();
                        } else {
                            let period_duration = current_time as i64 - start_ts;
                            // Make sure we don't give more credit than the current length of the
                            // period.
                            // TODO: make sure this still works if we prefetch blocks for contracts
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
                    SmartContractEvent::ConsumptionReportReceived(report) => {
                        let contract = match contracts.get_mut(&report.contract_id) {
                            Some(contract) => contract,
                            None => panic!(
                                "can't report consumption for unknown contract {} in block {}",
                                report.contract_id, block.height
                            ),
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
                        if report.timestamp as i64 <= contract.last_report_ts {
                            // Silently ignore reports out of order
                            continue;
                        }

                        // If report ts predates start we just memorize it but don't give credit.
                        if (report.timestamp as i64) < start_ts {
                            contract.last_reported_nru = report.nru;
                            contract.last_report_ts = report.timestamp as i64;
                            continue;
                        }

                        let time_elapsed = report.timestamp - contract.last_report_ts as u64;
                        node.capacity_consumption.cru += (report.cru * time_elapsed) as u128;
                        node.capacity_consumption.mru += (report.mru * time_elapsed) as u128;
                        node.capacity_consumption.hru += (report.hru * time_elapsed) as u128;
                        node.capacity_consumption.sru += (report.sru * time_elapsed) as u128;
                        node.capacity_consumption.ips += contract.ips as u64 * time_elapsed;
                        // Need to detect if the counter for nru was reset
                        let nru_used = if contract.last_reported_nru > report.nru {
                            // Counter reset
                            report.nru
                        } else {
                            report.nru - contract.last_reported_nru
                        };
                        node.capacity_consumption.nru += nru_used;

                        contract.last_reported_nru = report.nru;
                        contract.last_report_ts = report.timestamp as i64;
                    }
                    SmartContractEvent::ContractCreated(contract) => {
                        // we only care about node contracts
                        if let ContractData::NodeContract(nc) = contract.contract_type {
                            contracts.insert(
                                contract.contract_id,
                                Contract {
                                    contract_id: contract.contract_id,
                                    node_id: nc.node_id,
                                    last_report_ts: block.timestamp.timestamp(),
                                    last_reported_nru: 0,
                                    ips: nc.public_ips,
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

    let period_duration = (end_ts - start_ts) as u64;

    // TODO: add consumption stats
    println!("node_id,twin_id,farm_id,measured_uptime,CU,SU,NU,USD reward,TFT reward,TFT price on connect,cru,cru used,mru,mru used,hru,hru used,sru,sru used,IP used,DIY state,violation");
    for (_, node) in nodes {
        let (cu, su, nu) = node.cloud_units_permill();
        let musd = node.node_payout_musd();
        let tft = node.node_payout_tft_units();
        let cru_used = (node.capacity_consumption.cru / period_duration as u128) as u64;
        let mru_used = (node.capacity_consumption.mru / period_duration as u128) as u64;
        let hru_used = (node.capacity_consumption.hru / period_duration as u128) as u64;
        let sru_used = (node.capacity_consumption.sru / period_duration as u128) as u64;
        println!(
            "{},{},{},{},{},{},{},{},{},{},{},{:.2}%,{},{:.2}%,{},{:.2}%,{},{:.2}%,{:.2} hours,{},{}",
            node.id,
            node.twin_id,
            node.farm_id,
            if let Some((_, _, uptime)) = node.uptime_info {
                uptime
            } else {
                0
            },
            format_args!("{}.{:06}", cu / ONE_MILL as u64, cu % ONE_MILL as u64),
            format_args!("{}.{:06}", su / ONE_MILL as u64, su % ONE_MILL as u64),
            format_args!("{}.{:06}", nu / ONE_MILL as u64, nu % ONE_MILL as u64),
            format_args!("{}.{:03}", musd / 1_000, musd % 1_000),
            format_args!("{}.{:07}", tft / UNITS_PER_TFT, tft % UNITS_PER_TFT),
            format_args!(
                "{}.{:03}",
                node.connection_price / 1_000,
                node.connection_price % 1_000
            ),
            node.resources.cru,
            cru_used as f64 * 100. / node.resources.cru as f64,
            node.resources.mru,
            mru_used as f64 * 100. / node.resources.mru as f64,
            node.resources.hru,
            hru_used as f64 * 100. / node.resources.hru as f64,
            node.resources.sru,
            sru_used as f64 * 100. / node.resources.sru as f64,
            node.capacity_consumption.ips as f64 / 3600.,
            if let CertificationType::Certified = node.certification_type {
                "CERTIFIED"
            } else {
                "DIY"
            },
            if let Some(violation) = node.first_uptime_violation {
                violation
            } else {
                0
            }
        );
    }
}

fn block_import<P>(
    client: SharedClient<P>,
    start: BlockNumber,
    end: BlockNumber,
) -> mpsc::Receiver<MintingBlock>
where
    P: Pair,
    MultiSignature: From<P::Signature>,
{
    let mut receivers = Vec::with_capacity(RPC_THREADS);

    for i in 0..RPC_THREADS {
        let (tx, rx) = mpsc::sync_channel(PRE_FETCH);
        receivers.push(rx);
        let client = client.clone();
        let mut window = Window::at_height(client, start + i as u32)
            .unwrap()
            .unwrap();
        std::thread::spawn(move || loop {
            let height = window.height().unwrap();
            if height > end {
                break;
            }
            let timestamp = window.date().unwrap();
            let events = window.events().unwrap();
            tx.send(MintingBlock {
                height,
                timestamp,
                events,
            })
            .unwrap();
            window = window.advance_by(RPC_THREADS as u32).unwrap().unwrap();
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
    created: u64,
    certification_type: CertificationType,
    // (last ping, last reported uptime, total uptime).
    uptime_info: Option<(i64, u64, u64)>,
    // Block where the first uptime report violation was detected, if any.
    first_uptime_violation: Option<u32>,
    connected: NodeConnected,
    // TFT price expressed in USD at time of connection. Price is expressed in mUSD (3 digits
    // precision). I.e. 1 USD => 1000.
    connection_price: u64,
    // capacity consumed by workloads over a period.
    capacity_consumption: TotalConsumption,
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
            self.resources.cru as u128 * GIB * ONE_MILL,
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
    fn node_payout_musd(&self) -> u64 {
        let (cu, su, nu) = self.cloud_units_permill();
        let cu_reward = cu * CU_REWARD_MUSD;
        let su_reward = su * SU_REWARD_MUSD;
        let nu_reward = nu * NU_REWARD_MUSD;
        // Recall that IP usage is actually in seconds. Multiply the seconds of IP usage with the
        // hourly reward, then divide by 3600 seconds/hour. This prevents issues with low usage.
        let ip_reward = self.capacity_consumption.ips * IP_REWARD_MUSD / 3600;
        let base_payout = (cu_reward + su_reward + nu_reward) / ONE_MILL as u64 + ip_reward;
        if matches!(self.certification_type, CertificationType::Certified) {
            base_payout * 5 / 4
        } else {
            base_payout
        }
    }

    /// Calculate the TFT payout of the node in the minting period based on its cloud units and
    /// connection price. The payout is expressed in "units", where 1 TFT == 10_000_000 units.
    /// This also acconts for measured NU and Ip usage.
    fn node_payout_tft_units(&self) -> u64 {
        // connection price is in mUSD.
        self.node_payout_musd() * UNITS_PER_TFT / self.connection_price
    }
}

struct Contract {
    contract_id: u64,
    node_id: u32,
    // timestamp of last report. If not set, time when the contract was created.
    last_report_ts: i64,
    last_reported_nru: u64,
    ips: u32,
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
