# Offline xkcd

Ever wanted to show a friend a xkcd comic but you were offline?  
This application provides a local xkcd clone which is even searchable.

## Usage
1. Install and enable the **nightly** rust toolchain (preferred installation method is [rustup](https://rustup.rs/))
1. (optional) Execute the `cargo run ...` commands with `cargo run --release ...`
1. Clone the current xkcd data (execute this command whenever you want to update the data)  
   `cargo run -- update`
1. Start the local server  
   `cargo run -- server`
1. Browse xkcd at [localhost:8080](http://localhost:8080) and have fun

## Configuration
All configuration options can also be found with `cargo run -- help` and `cargo run -- help <subcommand>`.

 - While downloading xkcd, you can specify the number of concurrent connections with `cargo run -- update -j <num>` (default is 4)
 - Set the server port with `-p <port>` (default is 8080)
 - Set the listening address with `-a <address>` (default is 0.0.0.0)

License
-------
Licensed under either of

 * [Apache License, Version 2.0](LICENSE-APACHE)
 * [MIT license](LICENSE-MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
