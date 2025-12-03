//! # Agenda RS
//!
//! A stateful, database-backed job queue for Rust, inspired by `AgendaJS`.
//!
//! # Example: Creating and Scheduling a Job
//!
//! This example demonstrates how to define a custom job with internal state,
//! register it with the agenda, and schedule it for execution.
//!
//! ```rust,no_run
//! use agenda_rs::prelude::*;
//! use std::str::FromStr;
//! use std::time::Duration;
//!
//! // 1. Define your Payload (Must be Serialize/Deserialize)
//! #[derive(Debug, Serialize, Deserialize)]
//! struct Message {
//!     from: String,
//!     to: String,
//!     content: String,
//! }
//!
//! // 2. Define your Job Struct (Can hold internal state, database pools, etc.)
//! struct MailSender {
//!     prefix: String,
//! }
//!
//! // 3. Implement the Job Trait
//! #[async_trait]
//! impl Job for MailSender {
//!     const NAME: &'static str = "send_email";
//!     type Payload = Message;
//!
//!     async fn run(&self, payload: Self::Payload) {
//!         // You can access internal state (self.prefix) and the payload
//!         println!("{}: Sending email from {} to {}",
//!             self.prefix, payload.from, payload.to
//!         );
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! #     // Hidden setup code to make the doc cleaner
//! #     let conn_str = std::env::var("DATABASE_URL").unwrap_or("postgres://postgres:postgres@localhost:5432".to_string());
//! #     let pool = sqlx::postgres::PgPoolOptions::new()
//! #         .connect(&conn_str).await?;
//! #
//!     // 4. Initialize Agenda
//!     let mut agenda = Agenda::new(pool, None).await?;
//!
//!     // 5. Register the Job Handler
//!     // We pass an instance of MailSender, allowing us to inject state (e.g., "Mailer v1")
//!     let mailer = MailSender { prefix: "Mailer v1".to_string() };
//!     agenda.register(mailer).await;
//!
//!     // 6. Schedule a Job
//!     let message = Message {
//!         from: "alice@example.com".into(),
//!         to: "bob@example.com".into(),
//!         content: "Hello World".into(),
//!     };
//!
//!     // Run every minute ("0 * * * * *")
//!     let schedule = Schedule::from_str("0 * * * * *").unwrap();
//!     
//!     agenda.schedule::<MailSender>(schedule, message).await?;
//!
//!     // 7. Start the Agenda Runner (blocking)
//!     // In a real app, you might want to spawn this: tokio::spawn(agenda.start());
//!     // agenda.start().await;
//!    Ok(())
//! }
//! ```

mod agenda;
mod error;
mod job;

pub mod prelude {
    pub use crate::Agenda;
    pub use crate::AgendaError;
    pub use crate::Job;
    pub use async_trait::async_trait;
    pub use cron::Schedule;
    pub use serde::{Deserialize, Serialize};
}

pub use sqlx;

pub use agenda::Agenda;
pub use error::AgendaError;
pub use job::Job;
