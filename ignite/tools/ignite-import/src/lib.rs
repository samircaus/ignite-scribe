//! `ignite-import`: build Ignite notebooks from a small page-spec JSON.
//!
//! The tool is just another sync **replica**. It turns a spec into ordinary
//! [`sync_protocol::SyncOperationV1`] envelopes (`CreateNotebook`, `CreatePage`, `AddStroke`,
//! `AddText`), validates every one locally with the same code the server uses, uploads any paper
//! backgrounds as content-addressed blobs, and posts the envelopes to a `scribe-sync-server`.
//! The Kindle then pulls the notebook through its normal inbound sync path.
//!
//! It never edits or deletes existing content: every operation it emits is additive.
//!
//! Module map: [`spec`] (input format), [`font`] (built-in single-line stroke font), [`paper`]
//! (gray8 page backgrounds), [`build`] (spec → envelopes), [`client`] (HTTP + replica state),
//! [`preview`] (SVG preview, no device or server needed).

pub mod build;
pub mod client;
pub mod font;
pub mod layout;
pub mod manage;
pub mod note;
pub mod paper;
pub mod preview;
pub mod spec;
