//! Message-publishing abstraction for ingested documents.
//!
//! The ingestion service is decoupled from any specific message broker via the
//! [`DocumentPublisher`] trait. This keeps the library buildable and testable
//! without external infrastructure: unit and integration tests use
//! [`InMemoryPublisher`], while production deployments enable the
//! `nats-jetstream` feature to use [`NatsJetStreamPublisher`].

use async_trait::async_trait;
use std::sync::Mutex;

/// Error type returned by publishers. Boxed so that backend-specific error
/// types (NATS, etc.) can be surfaced without leaking their concrete types.
pub type PublishError = Box<dyn std::error::Error + Send + Sync>;

/// A transport that can publish an encoded document to a named subject.
#[async_trait]
pub trait DocumentPublisher: Send + Sync {
    /// Publish `payload` (an encoded [`crate::proto::spec_to_proof::v1::SpecDocument`])
    /// to `subject`, returning once the broker has acknowledged the message.
    async fn publish(&self, subject: &str, payload: &[u8]) -> Result<(), PublishError>;
}

/// An in-memory publisher that records every published message.
///
/// This is used by the test suite (so the ingestion pipeline can be exercised
/// with no running broker) and can also back the binary when it is compiled
/// without the `nats-jetstream` feature.
#[derive(Debug, Default)]
pub struct InMemoryPublisher {
    messages: Mutex<Vec<(String, Vec<u8>)>>,
}

impl InMemoryPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a snapshot of everything published so far.
    pub fn messages(&self) -> Vec<(String, Vec<u8>)> {
        self.messages.lock().expect("publisher mutex poisoned").clone()
    }

    /// Number of messages published to `subject`.
    pub fn count_for(&self, subject: &str) -> usize {
        self.messages
            .lock()
            .expect("publisher mutex poisoned")
            .iter()
            .filter(|(s, _)| s == subject)
            .count()
    }
}

#[async_trait]
impl DocumentPublisher for InMemoryPublisher {
    async fn publish(&self, subject: &str, payload: &[u8]) -> Result<(), PublishError> {
        self.messages
            .lock()
            .expect("publisher mutex poisoned")
            .push((subject.to_string(), payload.to_vec()));
        Ok(())
    }
}

/// A NATS JetStream backed publisher (available with the `nats-jetstream`
/// feature). Publishes are confirmed against the JetStream acknowledgement so
/// that a failure to persist propagates back to the caller.
#[cfg(feature = "nats-jetstream")]
pub struct NatsJetStreamPublisher {
    context: async_nats::jetstream::Context,
}

#[cfg(feature = "nats-jetstream")]
impl NatsJetStreamPublisher {
    /// Connect to a NATS server at `url` (e.g. `nats://localhost:4222`) and
    /// build a JetStream context.
    pub async fn connect(url: &str) -> Result<Self, PublishError> {
        let client = async_nats::connect(url).await?;
        Ok(Self {
            context: async_nats::jetstream::new(client),
        })
    }

    /// Build a publisher from an existing JetStream context.
    pub fn from_context(context: async_nats::jetstream::Context) -> Self {
        Self { context }
    }
}

#[cfg(feature = "nats-jetstream")]
#[async_trait]
impl DocumentPublisher for NatsJetStreamPublisher {
    async fn publish(&self, subject: &str, payload: &[u8]) -> Result<(), PublishError> {
        let ack = self
            .context
            .publish(subject.to_string(), bytes::Bytes::copy_from_slice(payload))
            .await?;
        // Wait for the JetStream acknowledgement so the caller learns about
        // storage failures rather than silently assuming success.
        ack.await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_publisher_records_messages() {
        let publisher = InMemoryPublisher::new();
        publisher.publish("spec-documents.jira", b"one").await.unwrap();
        publisher.publish("spec-documents.jira", b"two").await.unwrap();
        publisher.publish("spec-documents.confluence", b"three").await.unwrap();

        assert_eq!(publisher.count_for("spec-documents.jira"), 2);
        assert_eq!(publisher.count_for("spec-documents.confluence"), 1);
        assert_eq!(publisher.messages().len(), 3);
    }
}
