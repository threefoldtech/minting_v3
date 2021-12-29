# Minting V3

This repo contains the code for the off chain minting for grid V3, using
the V3 tokenomics. There are a couple of reasons as to why this is a
separate repo, and is not included in the [minting v2] repo. A couple of
important ones are:

- The v2 minting was designed to work against the existing v2
	infrastructure and its limitations, which are no longer relevant.
	Most importantly, the v2 explorers only keep track of active state,
	not historic state. This necessitated the development of an
	aggregator, which periodically pulled data from the explorer and
	saved it locally.
- The v2 data model for nodes and farms is different to the v3 one.
	While it is similar enough, there are already changes in that
	codebase to coerce v3 data structures to v2 ones to interoperate
	with the existing v2 logic. If we now change those structures to
	also be valid for v3, a lot of code paths will be created which in
	turn leads to a lot of potential for subtle bugs.
- As highlighted in the previous point, there are already a few
	hacks/patches in the existing code base. These all need to be
	maintained/tested/accounted for when writing new code. Therefore the
	advantage of being able to reuse part of the code is not worth it
	compared to rewriting everything (if this ends up being written in
	go, we can still reuse part of the logic anyhow).
- Other reasons


## New design

As explained, a limitation was lack of historic data in the public
explorer. In v3, all data is saved in the blockchain, so we don't need
to have a local DB with periodic syncs from the explorer. This
eliminates a large part of the complexity from the v2 setup. To simplify
things, we can still follow the v2 pattern of payment cycles, though
this is an arbitrary decision. A single program should be sufficient to
generate payout files based on a time range and an archive node.

It is possible that iterating over the entire chain will be costly,
although running time of the application is not really an issue. In this
case, after every run, a snapshot of the current state could be taken
and written to disk. As the application is deterministic, this snapshot
can easily be regenerated. Additional mechanisms such as uploading the
snapshot and including a hash of it in  "receipt file" of sorts (should
we start from a snapshot) can be employed, to allow everyone to
validate output.

For payout calculation, we can merge the steps of extracting data and
calculating payouts. It suffices to keep a list of all known nodes on
the network in memory, and update stats for a particular node as blocks
are iterated.

Payouts will still happen on the stellar network, as devnet uses "fake"
tokens which don't hold value.

A benefit of this design is that we don't need a particular piece of
infrastructure to be started in advance to collect data. The code can be
written and tested, and when it is done, we can simply mint using the
chain as data source. Since all data is stored forever (in archive
nodes), there is technically not even a requirement to finish this
before a certain data (as we can go back arbitrarily far in a network
history), other than people who expect to get their tokens regularly.

[minting v2]: https://github.com/threefoldtech/minting_v2
