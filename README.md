# minting_v3

minting code for grid v3, using v3 tokenomics

## description of process

- TFChain is our blockchain has all details about capacity provided by the nodes
- TFChain is used to track uptime
- Zero-OS reports to TFChain
- Additional Auditing Code will be added in v4 (special code generated at runtime verifies) using security primitives on motherboards
- The code in this repo uses the information from the blockchain to calculate the to be minted TFT
- A proof of what needs to be minted and why is created, this gets sent to our guardians
- The guardians need to double check the execution and the minting report (this is like a human check on the process)
- The guardians need to sign, only when consensus achieved the minting as suggested will happen (this allows human to check code)

> Its important to understant that our Blockchain tracks the capacity and uptime and is the source for the minting.
