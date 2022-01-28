use chrono::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use std::{collections::BTreeMap, sync::mpsc};
use tfchain_client::{
    client::{Client, MultiSignature, Pair, SharedClient},
    events::{TFGridEvent, TfchainEvent},
    types::{BlockNumber, CertificationType, Location, Node, Resources},
    window::Window,
};

const RPC_THREADS: usize = 100;
const PRE_FETCH: usize = 5;
const UPTIME_GRACE_PERIOD_SECONDS: i64 = 60;

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
                },
            )
        })
        .collect();
    println!("Found {} existing nodes", nodes.len());
    // Grab existing contracts

    println!("Setup block import pipeline");
    let blocks = end_block - start_block + 1;
    let block_import = block_import(client, start_block, end_block);

    println!("Block import pipeline finished");
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
                            // + a host of ther issues, we will allow a node to report uptime with
                            // "grace peirod" of a minute or so in either direction.
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
                                // within the expected timeframe. If nodes boot, send uptime, then
                                // immediatly reboot that is possible. In those cases, handle that
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

                            // We should hae handled all cases. Make this explicit here.
                            unreachable!();
                        } else {
                            let period_duration = current_time as i64 - start_ts;
                            // Make sure we don't give more credit than the current length of the
                            // period.
                            // TODO: make sure this stil works if we prefetch blocks for contracts
                            let up_in_period =
                                std::cmp::min(period_duration as u64, reported_uptime);
                            // Save uptime info
                            node.uptime_info =
                                Some((current_time as i64, reported_uptime, up_in_period));
                        }
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

    println!("node_id,twin_id,farm_id,measured_uptime,cru,mru,hru,sru,violation");
    for (_, node) in nodes {
        println!(
            "{},{},{},{},{},{},{},{},{}",
            node.id,
            node.twin_id,
            node.farm_id,
            if let Some((_, _, uptime)) = node.uptime_info {
                uptime
            } else {
                0
            },
            node.resources.cru,
            node.resources.mru,
            node.resources.hru,
            node.resources.sru,
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
    // Block where the first uptime reprot violation was detected, if any.
    first_uptime_violation: Option<u32>,
    connected: NodeConnected,
}
