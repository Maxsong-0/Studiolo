//! 经 capability 校验的工具与模型中继接缝：含 M0 内存适配器与持久化真实中继 `DurableRelay`。

mod admission;
mod call_admit;
mod call_table;
mod call_types;
mod capability;
mod durable_relay;
mod error;
mod execution;
mod gateway;
mod gateway_audit;
#[cfg(unix)]
mod relay_remote;
mod relay_request;
mod relay_retention;
mod relay_store;
mod relay_tools;
mod relay_transport;
mod relay_types;
mod relay_unix;
#[cfg(unix)]
pub use relay_remote::{RemoteRelayTransport, serve_key_holder_connection};

pub use call_table::MAX_RETAINED_RESULTS;
pub use call_types::{CallId, ModelRequest, ModelResponse, ToolCall, ToolResult};
pub use capability::{
    AttemptGuard, AttemptSnapshot, Clock, JobCapability, ManualClock, MemoryAttemptGuard,
    PermissionLayers,
};
pub use durable_relay::{DurableRelay, RelayServices};
pub use error::{AttemptGuardError, GatewayError};
pub use gateway::{Gateway, MemoryGateway};
pub use relay_transport::{HttpsRelayTransport, RelayTransport};
pub use relay_types::{
    ModelEgress, ProviderCredential, RelayError, RelayGrant, RelayLimits, RelayPolicyGuard,
    RelayProvider, RelayPublisher, RelayResponse, RelaySource, RelayTicket,
};
#[cfg(unix)]
pub use relay_unix::serve_relay_connection;
#[cfg(test)]
mod wait_probe;
#[cfg(test)]
mod waiting_tests;
