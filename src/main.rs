use chrono::prelude::*;
use indicatif::ProgressBar;
use std::sync::mpsc;
use tfchain_client::{
    client::{Client, MultiSignature, Pair, SharedClient},
    events::TfchainEvent,
    types::BlockNumber,
    window::Window,
};

const RPC_THREADS: usize = 100;
const PRE_FETCH: usize = 5;

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
    // Grab existing contracts

    println!("Setup block import pipeline");
    let blocks = end_block - start_block + 1;
    let block_import = block_import(client, start_block, end_block);

    println!("Block import pipeline finished");
    let bar = ProgressBar::new(blocks as u64);
    let mut last_height = start_block - 1;
    for block in block_import {
        assert_eq!(block.height, last_height + 1);

        // update last height for sanity check
        last_height = block.height;
        // finally update progress bar
        bar.inc(1);
    }

    bar.finish();
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
