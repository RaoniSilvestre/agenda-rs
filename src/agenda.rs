use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use chrono_tz::Tz;
use serde_json::Value;
use sqlx::{Pool, Postgres, prelude::FromRow};
use tokio::{sync::RwLock, time::sleep};

use crate::{
    error::{AgendaError, ErrorKind},
    job::{Job, JobRunner},
};

/// The primary entry point for managing background jobs.
///
/// `Agenda` is responsible for the full lifecycle of the job queue:
/// * **Registering** job handlers via [`Agenda::register`].
/// * **Scheduling** new job instances via [`Agenda::schedule`].
/// * **Running** the background processing loop via [`Agenda::start`].
///
/// This struct is cheap to [`Clone`] and is designed to be shared across threads
/// (e.g., passing it to your web server state).#[derive(Clone)]
#[derive(Clone)]
pub struct Agenda {
    conn: Pool<Postgres>,
    table_name: String,
    timezone: Tz,
    jobs: Arc<RwLock<HashMap<String, Box<dyn JobRunner>>>>,
}

impl Agenda {
    /// Creates a new `Agenda` instance and initializes the required database schema.
    ///
    /// This function will automatically create the jobs table and necessary indexes if they
    /// do not already exist. It also fetches the database's current `TimeZone` configuration
    /// to ensure accurate scheduling.
    ///
    /// # Arguments
    ///
    /// * `conn` - A connection pool to the `PostgreSQL` database.
    /// * `table_name` - Optional custom name for the jobs table. Defaults to `"stateful_cron_jobs"`.
    ///
    /// # Panics
    ///
    /// Panics if `table_name` contains non-alphanumeric characters (except underscores).
    /// This is a security measure to prevent SQL injection, as the table name cannot be parameterized.
    ///
    /// # Errors
    ///
    /// Returns an [`AgendaError`] if:
    /// * The database connection fails.
    /// * The table or index creation queries fail.
    /// * The database timezone cannot be fetched or parsed.
    pub async fn new(conn: Pool<Postgres>, table_name: Option<&str>) -> Result<Self, AgendaError> {
        let table = table_name.unwrap_or("stateful_cron_jobs");

        assert!(
            table.chars().all(|c| c.is_alphanumeric() || c == '_'),
            "Table name must be alphanumeric and underscores only"
        );

        let create_table_query = format!(
            r"
            CREATE TABLE IF NOT EXISTS {table} (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                job_name TEXT NOT NULL,
                cron_expression TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                next_run_at TIMESTAMPTZ NOT NULL,
                is_locked BOOLEAN NOT NULL DEFAULT false,
                payload JSONB,
                last_error TEXT
            );
            "
        );

        sqlx::query(&create_table_query).execute(&conn).await?;

        let create_index_query = format!(
            "CREATE INDEX IF NOT EXISTS idx_{table}_poll ON {table} (next_run_at, is_locked);"
        );

        sqlx::query(&create_index_query).execute(&conn).await?;

        let tz_string: String = sqlx::query_scalar("SHOW TimeZone").fetch_one(&conn).await?;

        let tz = Tz::from_str(&tz_string)?;

        Ok(Self {
            timezone: tz,
            table_name: table.to_string(),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            conn,
        })
    }

    /// Registers a handler for a specific job type.
    ///
    /// This stores the `job` instance in an internal registry. When the Agenda runner finds
    /// a scheduled task in the database with a matching [`Job::NAME`], it will use this
    /// specific instance to execute it.
    ///
    /// # Arguments
    ///
    /// * `job` - The struct implementing the [`Job`] trait. This instance is long-lived
    ///   and can hold state (like database pools or API keys) that is reused
    ///   across executions.
    pub async fn register<J>(&mut self, job: J)
    where
        J: Job + 'static,
    {
        let mut jobs = self.jobs.write().await;

        jobs.insert(J::NAME.to_string(), Box::new(job));
    }

    /// Cancels and deletes all scheduled instances of a specific job type.
    ///
    /// This removes every row in the database matching [`Job::NAME`], effectively
    /// wiping out the schedule and history for this job.
    ///
    /// # Errors
    ///
    /// Returns an [`AgendaError`] if the database query fails.
    pub async fn delete<J: Job>(&mut self) -> Result<(), AgendaError> {
        let query = format!("DELETE FROM {} WHERE job_name = $1", self.table_name);

        sqlx::query(&query)
            .bind(J::NAME.to_string())
            .execute(&self.conn)
            .await?;

        Ok(())
    }

    /// Schedules a new task for the specified Job type.
    ///
    /// This serializes the payload and inserts a new row into the database. The execution
    /// time is calculated immediately based on the provided `cron` schedule and the
    /// `Agenda`'s configured timezone.
    ///
    /// # Panics
    ///
    /// Panics if the `cron` schedule cannot produce a valid upcoming date.
    /// (This is practically impossible with standard cron expressions).
    ///
    /// # Errors
    ///
    /// Returns an [`AgendaError`] if:
    /// * The `payload` cannot be serialized to JSON.
    /// * The database insertion fails.
    pub async fn schedule<J: Job>(
        &self,
        cron: cron::Schedule,
        payload: J::Payload,
    ) -> Result<(), AgendaError> {
        let payload = serde_json::to_value(payload)?;

        let next_run_at = cron.upcoming(self.timezone).next().unwrap();

        let query = format!(
            "INSERT INTO {} (job_name, cron_expression, next_run_at, payload) VALUES($1, $2, $3, $4)",
            self.table_name
        );

        sqlx::query(&query)
            .bind(J::NAME.to_string())
            .bind(cron.to_string())
            .bind(next_run_at)
            .bind(serde_json::json!(payload))
            .execute(&self.conn)
            .await?;

        Ok(())
    }

    async fn execute(&self, name: &str, payload: Value) -> Result<(), AgendaError> {
        let jobs = self.jobs.read().await;

        match jobs.get(name) {
            None => Err(AgendaError(ErrorKind::JobNotFound)),
            Some(job) => job.run_erased(payload).await,
        }
    }

    /// Starts the job processor loop.
    ///
    /// This enters an infinite loop that continuously polls the database for unlocked jobs
    /// that are due for execution.
    ///
    /// * If a job is found, it is locked and executed in a separate `tokio::spawn` task.
    /// * If no jobs are found, the runner sleeps for a short duration to conserve resources.
    /// * If a database error occurs, it pauses briefly before retrying.
    ///
    /// # Blocking Behavior
    ///
    /// This function **does not return**. It is designed to run indefinitely as a background worker.
    /// If you are integrating this into a web server, you should spawn it in a separate task
    /// so it does not block your main server loop:
    ///
    /// ```rust,no_run
    /// # use agenda_rs::Agenda;
    /// # async fn run(agenda: Agenda) {
    /// // Correct usage in a web app:
    /// tokio::spawn(async move {
    ///     agenda.start().await;
    /// });
    /// # }
    /// ```
    pub async fn start(self) {
        println!("Agenda runner started...");

        loop {
            match self.fetch_next_job().await {
                Ok(Some(job)) => {
                    let worker = self.clone();

                    tokio::spawn(async move {
                        match worker.process_job(job).await {
                            Ok(()) => println!("Job finished sucessfully"),
                            Err(e) => eprintln!("Deu ruim! {e:?}"),
                        }
                    });
                }
                Ok(None) => {
                    sleep(Duration::from_secs(15)).await;
                }
                Err(e) => {
                    eprintln!("❌ Database error polling jobs: {e:?}");
                    sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// Finds a job that is due in the next 15 seconds AND locks it atomically.
    async fn fetch_next_job(&self) -> Result<Option<JobRecord>, sqlx::Error> {
        let look_ahead = chrono::Utc::now() + chrono::Duration::seconds(15);

        let sql = format!(
            r"
            UPDATE {0}
            SET is_locked = true
            WHERE id = (
                SELECT id
                FROM {0}
                WHERE is_locked = false
                AND next_run_at <= $1
                ORDER BY next_run_at ASC
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            RETURNING id, job_name, cron_expression, next_run_at, payload
            ",
            self.table_name
        );

        sqlx::query_as::<_, JobRecord>(&sql)
            .bind(look_ahead)
            .fetch_optional(&self.conn)
            .await
    }

    /// Waiting, Execution, and Rescheduling logic
    async fn process_job(&self, job: JobRecord) -> Result<(), AgendaError> {
        let now = chrono::Utc::now();

        if job.next_run_at > now {
            let wait_time = (job.next_run_at - now).to_std().unwrap_or(Duration::ZERO);
            println!(
                "⏳ Job '{}' scheduled. Waiting {:?}...",
                job.job_name, wait_time
            );
            sleep(wait_time).await;
        }

        println!(
            "⚙️ Executing Job {} at the time {}",
            job.job_name,
            chrono::Utc::now()
        );
        self.execute(&job.job_name, job.payload.clone()).await?;

        if let Err(e) = self.reschedule_job(&job).await {
            eprintln!("❌ Failed to reschedule job {}: {:?}", job.job_name, e);
        }

        Ok(())
    }

    async fn reschedule_job(&self, job: &JobRecord) -> Result<(), AgendaError> {
        let schedule = cron::Schedule::from_str(&job.cron_expression)?;

        let next_run = schedule.after(&job.next_run_at).next().unwrap();

        let sql = format!(
            "UPDATE {} SET is_locked = false, next_run_at = $1, last_error = NULL WHERE id = $2",
            self.table_name
        );

        sqlx::query(&sql)
            .bind(next_run)
            .bind(job.id)
            .execute(&self.conn)
            .await?;

        println!("Create next run for {} at {}", job.job_name, next_run);
        Ok(())
    }
}

#[derive(FromRow)]
struct JobRecord {
    id: uuid::Uuid,
    job_name: String,
    cron_expression: String,
    next_run_at: chrono::DateTime<chrono::Utc>,
    payload: Value,
}

use std::fmt;

impl fmt::Debug for Agenda {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Start the struct builder
        let mut d = f.debug_struct("Agenda");

        d.field("conn", &self.conn);
        d.field("table_name", &self.table_name);
        d.field("timezone", &self.timezone);

        d.finish_non_exhaustive()
    }
}
