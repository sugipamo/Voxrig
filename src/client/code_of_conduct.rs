//! Received server code of conduct, automatically accepted during configuration.
use super::ObservedValue;
use crate::Result;

impl super::Client {
    /// Latest received Java 1.21.11 code-of-conduct text, or `None` before receipt.
    /// The adapter automatically replies with acceptance during configuration.
    /// This is a received fact, not proof that the reply reached the server.
    /// Retains one string (at most 32767 UTF-16 code units); a newer receipt
    /// replaces it. Reconfiguration and disconnect preserve the last receipt.
    /// Java 1.16.1 returns `ErrorKind::Unsupported`.
    pub async fn code_of_conduct(&self) -> Result<Option<ObservedValue<String>>> {
        super::dispatch!(&self.adapter, a => super::adapter::EventOps::code_of_conduct(a).await)
    }
}
