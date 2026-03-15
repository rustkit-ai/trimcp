#![deny(clippy::all)]

mod cache;
mod compress;
mod config;
mod error;
mod metrics;
mod protocol;
mod proxy;
mod transport;

fn main() {
    println!("rustkit-mcp");
}
