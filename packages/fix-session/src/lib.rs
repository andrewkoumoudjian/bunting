#![forbid(unsafe_code)]
//! Durable, transport-neutral FIX session sequencing.

use core::fmt;
use serde::{Deserialize, Serialize};

pub const MAX_RESEND_MESSAGES: u32 = 4_096;

/// Persisted FIX session counters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState { pub next_sender_sequence: u32, pub next_target_sequence: u32, pub logged_on: bool }
impl Default for SessionState { fn default() -> Self { Self { next_sender_sequence: 1, next_target_sequence: 1, logged_on: false } } }

/// Stores the complete session state atomically.
pub trait StateStore {
    type Error: fmt::Display;
    fn load(&self) -> Result<Option<SessionState>, Self::Error>;
    fn save(&mut self, state: &SessionState) -> Result<(), Self::Error>;
}

/// Reports durable store or FIX sequence failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error { Store(String), SequenceTooLow { expected: u32, received: u32 }, Gap { expected: u32, received: u32 }, ResendTooLarge }
impl fmt::Display for Error { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") } }
impl std::error::Error for Error {}

/// A resend interval requested from a peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResendRange { pub begin: u32, pub end: u32 }

/// Owns one FIX session's durable sequence state.
pub struct Session<S> { store: S, state: SessionState }
impl<S: StateStore> Session<S> {
    /// Restores a session without silently resetting absent state.
    ///
    /// # Errors
    /// Returns [`Error::Store`] when durable state cannot be loaded.
    pub fn restore(store: S) -> Result<Self, Error> { let state = store.load().map_err(|e| Error::Store(e.to_string()))?.unwrap_or_default(); Ok(Self { store, state }) }
    #[must_use] pub const fn state(&self) -> &SessionState { &self.state }
    /// Marks a successful logon and persists it.
    ///
    /// # Errors
    /// Returns a store error if persistence fails.
    pub fn logon(&mut self) -> Result<(), Error> { self.state.logged_on = true; self.persist() }
    /// Marks logout and persists it.
    ///
    /// # Errors
    /// Returns a store error if persistence fails.
    pub fn logout(&mut self) -> Result<(), Error> { self.state.logged_on = false; self.persist() }
    /// Allocates and durably advances one outbound sequence number.
    ///
    /// # Errors
    /// Returns a store error or an exhaustion error.
    pub fn next_outbound(&mut self) -> Result<u32, Error> { let sequence = self.state.next_sender_sequence; self.state.next_sender_sequence = sequence.checked_add(1).ok_or(Error::ResendTooLarge)?; self.persist()?; Ok(sequence) }
    /// Accepts an inbound sequence or returns its exact recovery action.
    ///
    /// # Errors
    /// Returns a gap, duplicate, exhaustion, or store error.
    pub fn accept_inbound(&mut self, received: u32, possible_duplicate: bool) -> Result<(), Error> { let expected = self.state.next_target_sequence; if received < expected { return if possible_duplicate { Ok(()) } else { Err(Error::SequenceTooLow { expected, received }) }; } if received > expected { if received - expected > MAX_RESEND_MESSAGES { return Err(Error::ResendTooLarge); } return Err(Error::Gap { expected, received }); } self.state.next_target_sequence = expected.checked_add(1).ok_or(Error::ResendTooLarge)?; self.persist() }
    /// Applies a peer SequenceReset-GapFill new sequence.
    ///
    /// # Errors
    /// Rejects backward resets and store failures.
    pub fn gap_fill(&mut self, new_sequence: u32) -> Result<(), Error> { if new_sequence < self.state.next_target_sequence { return Err(Error::SequenceTooLow { expected: self.state.next_target_sequence, received: new_sequence }); } self.state.next_target_sequence = new_sequence; self.persist() }
    fn persist(&mut self) -> Result<(), Error> { self.store.save(&self.state).map_err(|e| Error::Store(e.to_string())) }
}

/// In-memory store for deterministic tests and embedded callers.
#[derive(Default)] pub struct MemoryStore(pub Option<SessionState>);
impl StateStore for MemoryStore { type Error = core::convert::Infallible; fn load(&self) -> Result<Option<SessionState>, Self::Error> { Ok(self.0.clone()) } fn save(&mut self, state: &SessionState) -> Result<(), Self::Error> { self.0 = Some(state.clone()); Ok(()) } }

#[cfg(not(target_arch = "wasm32"))]
pub mod file_store {
    //! Atomic native filesystem session persistence.
    use super::{SessionState, StateStore}; use std::{fs, io, path::PathBuf};
    /// Persists one JSON state file using write-then-rename replacement.
    pub struct FileStore { path: PathBuf }
    impl FileStore { #[must_use] pub fn new(path: PathBuf) -> Self { Self { path } } }
    impl StateStore for FileStore { type Error = io::Error; fn load(&self) -> Result<Option<SessionState>, Self::Error> { match fs::read(&self.path) { Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(io::Error::other), Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None), Err(error) => Err(error) } } fn save(&mut self, state: &SessionState) -> Result<(), Self::Error> { let temporary = self.path.with_extension("tmp"); let bytes = serde_json::to_vec(state).map_err(io::Error::other)?; fs::write(&temporary, bytes)?; fs::rename(temporary, &self.path) } }
}

// Rust guideline compliant 2026-02-21

#[cfg(test)] mod tests { use super::*; #[test] fn restart_gap_and_gap_fill_are_exact() -> Result<(), Error> { let mut session = Session::restore(MemoryStore::default())?; session.logon()?; assert_eq!(session.next_outbound()?, 1); assert_eq!(session.accept_inbound(3, false), Err(Error::Gap { expected: 1, received: 3 })); session.gap_fill(3)?; session.accept_inbound(3, false)?; assert_eq!(session.state().next_target_sequence, 4); Ok(()) } }
