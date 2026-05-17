// THIS FILE IS MANUALLY AUTHORED TO MATCH THE WASM MODULE.
// Mirrors the `set_api_key(provider_id, api_key)` reducer added in ADR-2603300100 P2.2.

#![allow(unused, clippy::all)]
use spacetimedb_sdk::__codegen::{self as __sdk, __lib, __sats, __ws};

#[derive(__lib::ser::Serialize, __lib::de::Deserialize, Clone, PartialEq, Debug)]
#[sats(crate = __lib)]
pub(super) struct SetApiKeyArgs {
    pub provider_id: String,
    pub api_key: String,
}

impl From<SetApiKeyArgs> for super::Reducer {
    fn from(args: SetApiKeyArgs) -> Self {
        Self::SetApiKey {
            provider_id: args.provider_id,
            api_key: args.api_key,
        }
    }
}

impl __sdk::InModule for SetApiKeyArgs {
    type Module = super::RemoteModule;
}

#[allow(non_camel_case_types)]
/// Extension trait for access to the reducer `set_api_key`.
///
/// Implemented for [`super::RemoteReducers`].
pub trait set_api_key {
    /// Request that the remote module invoke the reducer `set_api_key`.
    fn set_api_key(&self, provider_id: String, api_key: String) -> __sdk::Result<()> {
        self.set_api_key_then(provider_id, api_key, |_, _| {})
    }

    fn set_api_key_then(
        &self,
        provider_id: String,
        api_key: String,
        callback: impl FnOnce(
                &super::ReducerEventContext,
                Result<Result<(), String>, __sdk::InternalError>,
            ) + Send
            + 'static,
    ) -> __sdk::Result<()>;
}

impl set_api_key for super::RemoteReducers {
    fn set_api_key_then(
        &self,
        provider_id: String,
        api_key: String,
        callback: impl FnOnce(
                &super::ReducerEventContext,
                Result<Result<(), String>, __sdk::InternalError>,
            ) + Send
            + 'static,
    ) -> __sdk::Result<()> {
        self.imp.invoke_reducer_with_callback(
            SetApiKeyArgs { provider_id, api_key },
            callback,
        )
    }
}
