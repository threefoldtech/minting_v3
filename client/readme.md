# TFchain client lib and cli

## building

`cargo build`

## Adding support for a new runtime version

First, ensure you have `subxt-cli` installed. The version should match the version of `subxt` declared in ./Cargo.toml, and can be installed with `cargo install`. For example,
if the version is declared as "0.27", you can install the matching `subxt-cli` with `cargo install -f --version ^0.27 subxt-cli`.

Next, download the metadata version from a substrate node, and save the encoded metadata in the ./artifacts directory. This is done with the `subxt-cli`. Example:
`subxt metadata --url wss://tfchain.test.grid.tf:443 > artifacts/v141.scale`. This will download the currently active metadata from tfchain.test.grid.tf (version 141).

After this, generate the code using the `subxt-cli`. Note that this is __technically__ not needed, as you can use a macro and point it to the scale file we just
downloaded, but this is easier to debug issues. Also, make sure the code is formatted properly: `subxt codegen --file artifacts/v141.scale | rustfmt --emit=stdout > src/runtimes/v141/runtime.rs`.

Finally, reexport the proper types and add the required decodes in ./src/dynamic.rs. You can check the existing code to see how this is done.

### Get Farm

`./target/debug/tfchain_cli farms 1`
