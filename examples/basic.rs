use std::{str::FromStr, time::Duration};

use agenda_rs::{Agenda, Job};
use async_trait::async_trait;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;

#[derive(Debug, Default)]
struct MailSender;

#[derive(Deserialize, Serialize, Debug)]
struct Message {
    from: String,
    to: String,
    content: String,
}

#[async_trait]
impl Job for MailSender {
    const NAME: &'static str = "send_email";

    type Payload = Message;

    async fn run(&self, payload: Self::Payload) {
        println!("This is my payload! {payload:?}",);
    }
}

#[tokio::main]
async fn main() {
    // Spin up a postgres
    let conn = PgPoolOptions::new()
        .max_connections(100)
        .min_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect("postgres://postgres:postgres@localhost:5432")
        .await
        .unwrap();

    let mut agenda = Agenda::new(conn, None).await.unwrap();

    // Fresh start
    agenda.delete::<MailSender>().await.unwrap();

    // Register in the agenda internals
    agenda.register(MailSender).await;

    let msg = Message {
        to: String::from("João"),
        from: String::from("Maria"),
        content: String::from("Hello little João!"),
    };

    // Schedule new execution
    agenda
        .schedule::<MailSender>(Schedule::from_str("0 * * * * *").unwrap(), msg)
        .await
        .unwrap();

    println!("Starting agenda.");
    // Start listening
    agenda.start().await;
}
