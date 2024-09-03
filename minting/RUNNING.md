# Minting flow

The minting code operates on "periods", which are a given length of time. The first
period started at the start of the minting. As a result of this, every period has
a predefined start and end date. Within this period, the minting aggregates certain
events in chronological order, and processes these to calculate the minting reward
for every node. It also keeps track of potential violations, and nodes which have
actual violations at the end of the period do not get any rewards.

Due to a number of people who filled in wrong addresses at the start of V3 minting,
there is a check to get all receipts of the previous minting period, verify on
stellar which of these have been paid, and make a new attempt to pay the ones which
haven't been paid. This happens only once per payout, so if you have a payment in
period 1 which did not succeed, in period 2 we will try to pay this amount to the
address specified on the farm __at the end of period 2__, but if this fails we
won't try again in period 3.

## Running the minting

### Prerequisites

In order to calculate minting output for a period, you will need the following:

- A binary compiled for this repo at the correct commit for that month.
- An archive node with exposed rpc port.
- Receipts from the previous minting cycle.

### Running

After compiling the binary from the correct commit, it can be run with 2 arguments.
The first is the minting period for which to calculate the payouts. The second
is the RPC address of the archive node to use. The receipts of the previous month
are expected in a directory `receipts/{period-1}`, where period is the period being
minted.

Once the minting is done, it will generate a log file, a file which details about
every node, and a file with all payouts that need to be done. Additionally, a directory
`receipts/{period}` will be created if it does not exist yet. The receipts will
be added to this directory (existing content won't be replaced, unless the name
collides with that of a receipt).
