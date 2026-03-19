/// trimcp — MCP proxy library
///
/// This crate can be used both as a CLI (`trimcp`) and as a library. The public
/// API exposes the core building blocks so you can embed the proxy logic in your
/// own applications.
///
/// # Key types
///
/// - [`config::Config`] — load / save proxy configuration from TOML
/// - [`proxy::Proxy`] — spawn an upstream MCP server and forward messages
/// - [`knowledge::KnowledgeStore`] — semantic response cache (rustkit-semantic)
/// - [`code_context::CodeContext`] — semtree code-context injection pipeline
/// - [`compress::Pipeline`] — text compression pipeline
/// - [`protocol`] — JSON-RPC types used by the MCP protocol
pub mod cache;
#[cfg(feature = "semtree")]
pub mod code_context;
pub mod compress;
pub mod config;
pub mod error;
pub mod knowledge;
pub mod metrics;
pub mod protocol;
pub mod proxy;
pub mod transport;

mod clients;
mod setup;
mod stats_store;
mod status;
mod cmd_knowledge;
mod cmd_stats;
