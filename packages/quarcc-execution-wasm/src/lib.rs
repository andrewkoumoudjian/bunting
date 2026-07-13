#![forbid(unsafe_code)]

use quarcc_execution_engine::{
    AuthoritativeVenueSnapshot, ExecutionActionBuffer, ExecutionConfig, ExecutionEngine,
    ExecutionIntent, ExecutionSnapshot, MarketObservation, NormalizedVenueReport,
    QuarccExecutionEngine,
};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;

fn decode<T: DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|error| JsValue::from_str(&error.to_string()))
}

fn encode<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(value).map_err(|error| JsValue::from_str(&error.to_string()))
}

fn failure(error: impl core::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[wasm_bindgen]
pub struct WasmExecutionEngine {
    inner: QuarccExecutionEngine,
}

#[wasm_bindgen]
#[allow(clippy::missing_errors_doc)]
impl WasmExecutionEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(config: JsValue) -> Result<Self, JsValue> {
        Ok(Self {
            inner: QuarccExecutionEngine::new(decode(config)?),
        })
    }

    pub fn submit_intent(&mut self, input: JsValue) -> Result<JsValue, JsValue> {
        let intent: ExecutionIntent = decode(input)?;
        let limit = self.inner.snapshot().config.max_actions_per_call;
        let mut output = ExecutionActionBuffer::with_limit(limit);
        self.inner
            .submit_intent(intent, &mut output)
            .map_err(failure)?;
        encode(&output.into_vec())
    }

    pub fn apply_market_data(&mut self, input: JsValue) -> Result<JsValue, JsValue> {
        let observation: MarketObservation = decode(input)?;
        let limit = self.inner.snapshot().config.max_actions_per_call;
        let mut output = ExecutionActionBuffer::with_limit(limit);
        self.inner
            .apply_market_data(&observation, &mut output)
            .map_err(failure)?;
        encode(&output.into_vec())
    }

    pub fn apply_report(&mut self, input: JsValue) -> Result<JsValue, JsValue> {
        let report: NormalizedVenueReport = decode(input)?;
        let limit = self.inner.snapshot().config.max_actions_per_call;
        let mut output = ExecutionActionBuffer::with_limit(limit);
        self.inner
            .apply_venue_report(&report, &mut output)
            .map_err(failure)?;
        encode(&output.into_vec())
    }

    pub fn reconcile(&mut self, input: JsValue) -> Result<JsValue, JsValue> {
        let snapshot: AuthoritativeVenueSnapshot = decode(input)?;
        let limit = self.inner.snapshot().config.max_actions_per_call;
        let mut output = ExecutionActionBuffer::with_limit(limit);
        self.inner
            .reconcile(&snapshot, &mut output)
            .map_err(failure)?;
        encode(&output.into_vec())
    }

    pub fn snapshot(&self) -> Result<JsValue, JsValue> {
        encode(&self.inner.snapshot())
    }

    pub fn restore(snapshot: JsValue) -> Result<WasmExecutionEngine, JsValue> {
        let snapshot: ExecutionSnapshot = decode(snapshot)?;
        Ok(Self {
            inner: QuarccExecutionEngine::restore(snapshot).map_err(failure)?,
        })
    }

    pub fn default_config() -> Result<JsValue, JsValue> {
        encode(&ExecutionConfig::default())
    }
}
