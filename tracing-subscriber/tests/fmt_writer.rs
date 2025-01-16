#![cfg(feature = "fmt")]

//! Tests for the [`MakeWriter`] implementation on [`Option<M>`].
//!
//! These tests exercise optional writers through a formatting subscriber rather
//! than calling the implementation directly. This covers the same
//! `make_writer_for` path used when formatting real events, including
//! composition with [`MakeWriterExt::and`].

use std::{
    io,
    sync::{Arc, Mutex},
};
use tracing_core::Metadata;
use tracing_subscriber::fmt::writer::{MakeWriter, MakeWriterExt, OptionalWriter};

// Clones share the same byte buffer, allowing the test to retain one handle
// while the formatting subscriber owns another.
#[derive(Clone, Debug, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[derive(Clone, Debug)]
struct MetadataAwareWriter(Buffer);

impl<'a> MakeWriter<'a> for MetadataAwareWriter {
    type Writer = OptionalWriter<Buffer>;

    // Deliberately return a disabled writer when no metadata is provided. This
    // makes the test below fail if `Option<M>::make_writer_for` incorrectly
    // delegates to `M::make_writer`.
    fn make_writer(&'a self) -> Self::Writer {
        OptionalWriter::none()
    }

    // Enable output only when the event metadata has the expected target.
    fn make_writer_for(&'a self, metadata: &Metadata<'_>) -> Self::Writer {
        if metadata.target() == "optional_target" {
            return OptionalWriter::some(self.0.clone());
        }

        OptionalWriter::none()
    }
}

#[test]
fn optional_make_writer_composes_with_required_writer() {
    let required = Buffer::default();
    let optional = Buffer::default();

    // `Some` adds the optional destination to the required destination.
    record_event(required.clone(), Some(optional.clone()));
    assert!(
        required.contents().contains("hello from both writers"),
        "the required writer should receive the event"
    );
    assert!(
        optional.contents().contains("hello from both writers"),
        "a writer wrapped in Some should receive the event"
    );

    // `None` disables only the optional destination. The required side of the
    // `Tee` must continue receiving events.
    let required = Buffer::default();
    record_event(required.clone(), None);
    assert!(
        required.contents().contains("hello from both writers"),
        "None should not disable the required writer"
    );
}

#[test]
fn optional_make_writer_forwards_metadata() {
    let output = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Some(MetadataAwareWriter(output.clone())))
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "optional_target", "metadata was forwarded");
    });

    assert!(
        output.contents().contains("metadata was forwarded"),
        "a writer wrapped in Some should receive the event metadata"
    );
}

fn record_event(required: Buffer, optional: Option<Buffer>) {
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(required.and(optional))
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("hello from both writers");
    });
}
