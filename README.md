# Ledger Minting

A token minting calculation engine that validates and computes token rewards based on blockchain-tracked resource capacity and uptime data.

## What this is

This repository implements a tokenomics model for calculating and distributing token rewards based on verified infrastructure contributions. It reads capacity and uptime data from a blockchain, validates the inputs, computes the reward allocations, and generates a cryptographic proof report. The proof report is submitted for multi-party human verification before any tokens are minted.

The process ensures transparent, verifiable token issuance tied to actual resource contributions.

## What this repository contains

- **Minting calculation logic** — reads capacity and uptime records from the blockchain and computes reward allocations.
- **Proof generation** — produces a detailed, auditable report showing exactly what should be minted and why.
- **Guardian verification flow** — the proof report is distributed to guardians who review and sign off on the computation before execution.

## Role in the stack

- A blockchain records all capacity provided by nodes and tracks their uptime.
- Node operating systems report capacity and uptime to the blockchain.
- The code in this repository uses the on-chain data as the single source of truth to calculate rewards.
- A proof of the intended minting is created and sent to guardians for consensus-based human review.
- Only after guardian consensus is achieved is the minting executed.

## Relation to ThreeFold

This technology is used within the ThreeFold ecosystem and was first deployed on the ThreeFold Grid. The component itself is designed as reusable infrastructure technology and should be understood by its technical function first, independent of any specific deployment.

## Ownership

This repository is owned and maintained by TF-Tech NV, a Belgian company responsible for the development and maintenance of this technology.

## License

This project is licensed under the Apache License 2.0 — see the [LICENSE](LICENSE) file for details.
