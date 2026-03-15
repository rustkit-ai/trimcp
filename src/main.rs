#![deny(clippy::all)]

mod cache;
mod compress;
mod config;
mod error;
mod metrics;
mod protocol;
mod proxy;

fn main() {
    println!("rustkit-mcp");
}
