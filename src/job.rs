use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::error::{AgendaError, ErrorKind};

/// Defines a background task that can be scheduled and executed by the Agenda.
///
/// The doc didn't compile the `#[async_trait]` macro for clarity.
///
/// # Implementing this Trait
///
/// To actually implement this trait, you need to add the `#[async_trait]` macro, so rust can
/// deal with lifetimes without bothering you.
///
/// ```rust
/// use agenda_rs::prelude::*;
///
/// #[derive(Debug, Serialize, Deserialize)]
/// struct MyPayload {
///     user_id: i32,
/// }
///
/// struct WelcomeEmail;
///
/// #[async_trait]
/// impl Job for WelcomeEmail {
///     const NAME: &'static str = "welcome_email";
///     type Payload = MyPayload;
///
///     // simply write 'async fn' here:
///     async fn run(&self, payload: Self::Payload) {
///         println!("Processing user: {}", payload.user_id);
///     }
/// }
/// ```
#[cfg_attr(not(doc), async_trait)]
pub trait Job: Send + Sync {
    /// A unique identifier for this job type used in the database.
    ///
    /// **Warning:** This name must be stable. Changing it will orphan existing
    /// scheduled jobs in the database (they will fail to find a matching runner).
    ///
    /// **Warning:** This name MUST be unique. The library was designed with one name per job. If
    /// the same name is used for more than one job, your job can be queued with the incorrect
    /// payload and fail the serialization.
    const NAME: &'static str;

    /// The data required to execute the job.
    ///
    /// This data is serialized to JSON and stored in the database when the job is scheduled.
    type Payload: DeserializeOwned + Serialize + Send;

    /// The logic to execute when the job runs.
    ///
    /// # Arguments
    ///
    /// * `payload` - The deserialized data associated with this specific job instance.
    async fn run(&self, payload: Self::Payload);
}

#[async_trait]
pub(super) trait JobRunner: Send + Sync {
    async fn run_erased(&self, payload: Value) -> Result<(), AgendaError>;
}

#[async_trait]
impl<T> JobRunner for T
where
    T: Job,
{
    async fn run_erased(&self, payload: Value) -> Result<(), AgendaError> {
        match serde_json::from_value(payload) {
            Ok(value) => {
                self.run(value).await;
                Ok(())
            }
            Err(e) => Err(AgendaError(ErrorKind::SerializationError {
                message: e.to_string(),
            })),
        }
    }
}
