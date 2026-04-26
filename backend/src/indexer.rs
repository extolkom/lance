use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::{debug, error, info, warn};

const DEFAULT_SOROBAN_RPC_URL: &str = "https://soroban-testnet.stellar.org";
const DEFAULT_IDLE_POLL_MS: u64 = 2_000;
const DEFAULT_RPC_RATE_LIMIT_MS: u64 = 250;
const DEFAULT_RPC_RETRY_ATTEMPTS: u32 = 4;
const DEFAULT_RPC_RETRY_INITIAL_BACKOFF_MS: u64 = 500;
const DEFAULT_RPC_RETRY_MAX_BACKOFF_MS: u64 = 5_000;
const DEFAULT_WORKER_RETRY_INITIAL_BACKOFF_MS: u64 = 1_000;
const DEFAULT_WORKER_RETRY_MAX_BACKOFF_MS: u64 = 60_000;

#[derive(Default)]
pub struct IndexerMetrics {
    pub last_processed_ledger: AtomicI64,
    pub last_network_ledger: AtomicI64,
    pub total_events_processed: AtomicU64,
    pub total_errors: AtomicU64,
    pub total_rpc_retries: AtomicU64,
    pub last_loop_duration_ms: AtomicU64,
    pub last_rpc_latency_ms: AtomicU64,
    pub last_batch_events_processed: AtomicU64,
    pub last_batch_rate_per_second: AtomicU64,
}

pub static INDEXER_METRICS: OnceLock<IndexerMetrics> = OnceLock::new();

pub fn metrics() -> &'static IndexerMetrics {
    INDEXER_METRICS.get_or_init(IndexerMetrics::default)
}

#[derive(Clone, Debug)]
struct RetryPolicy {
    max_attempts: u32,
    initial_backoff: Duration,
    max_backoff: Duration,
}

impl RetryPolicy {
    fn from_env(
        prefix: &str,
        default_attempts: u32,
        default_initial_ms: u64,
        default_max_ms: u64,
    ) -> Self {
        Self {
            max_attempts: read_env_u32(&format!("{prefix}_MAX_ATTEMPTS"), default_attempts).max(1),
            initial_backoff: Duration::from_millis(read_env_u64(
                &format!("{prefix}_INITIAL_BACKOFF_MS"),
                default_initial_ms,
            )),
            max_backoff: Duration::from_millis(read_env_u64(
                &format!("{prefix}_MAX_BACKOFF_MS"),
                default_max_ms.max(default_initial_ms),
            )),
        }
    }

    fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let factor = 2u128.saturating_pow(attempt);
        let raw_ms = self.initial_backoff.as_millis().saturating_mul(factor);
        let capped_ms = raw_ms.min(self.max_backoff.as_millis());
        Duration::from_millis(capped_ms as u64)
    }
}

#[derive(Clone, Debug)]
struct IndexerConfig {
    rpc_url: String,
    idle_poll_interval: Duration,
    rpc_rate_limit_interval: Duration,
    rpc_retry_policy: RetryPolicy,
    worker_retry_policy: RetryPolicy,
}

impl IndexerConfig {
    fn from_env() -> Self {
        Self {
            rpc_url: std::env::var("SOROBAN_RPC_URL")
                .or_else(|_| std::env::var("STELLAR_RPC_URL"))
                .unwrap_or_else(|_| DEFAULT_SOROBAN_RPC_URL.to_string()),
            idle_poll_interval: Duration::from_millis(read_env_u64(
                "INDEXER_IDLE_POLL_MS",
                DEFAULT_IDLE_POLL_MS,
            )),
            rpc_rate_limit_interval: Duration::from_millis(read_env_u64(
                "INDEXER_RPC_RATE_LIMIT_MS",
                DEFAULT_RPC_RATE_LIMIT_MS,
            )),
            rpc_retry_policy: RetryPolicy::from_env(
                "INDEXER_RPC_RETRY",
                DEFAULT_RPC_RETRY_ATTEMPTS,
                DEFAULT_RPC_RETRY_INITIAL_BACKOFF_MS,
                DEFAULT_RPC_RETRY_MAX_BACKOFF_MS,
            ),
            worker_retry_policy: RetryPolicy::from_env(
                "INDEXER_WORKER_RETRY",
                DEFAULT_RPC_RETRY_ATTEMPTS,
                DEFAULT_WORKER_RETRY_INITIAL_BACKOFF_MS,
                DEFAULT_WORKER_RETRY_MAX_BACKOFF_MS,
            ),
        }
    }
}

struct SorobanRpcClient {
    client: Client,
    config: IndexerConfig,
    last_request_started_at: Option<Instant>,
}

impl SorobanRpcClient {
    fn new(client: Client, config: IndexerConfig) -> Self {
        Self {
            client,
            config,
            last_request_started_at: None,
        }
    }

    async fn get_latest_ledger(&mut self) -> Result<i64> {
        let result = self.rpc_request("getLatestLedger", json!({})).await?;
        let sequence = result
            .get("sequence")
            .and_then(parse_i64)
            .ok_or_else(|| anyhow!("missing sequence in getLatestLedger response"))?;

        metrics()
            .last_network_ledger
            .store(sequence, Ordering::Relaxed);

        Ok(sequence)
    }

    async fn get_events(&mut self, start_ledger: i64) -> Result<EventsResponse> {
        let result = self
            .rpc_request(
                "getEvents",
                json!({
                    "startLedger": start_ledger,
                    "filters": []
                }),
            )
            .await?;

        let latest_network_ledger = result
            .get("latestLedger")
            .and_then(parse_i64)
            .unwrap_or(start_ledger.saturating_sub(1));

        metrics()
            .last_network_ledger
            .store(latest_network_ledger, Ordering::Relaxed);

        let events = result
            .get("events")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        Ok(EventsResponse {
            latest_network_ledger,
            events,
        })
    }

    async fn rpc_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        for attempt in 0..self.config.rpc_retry_policy.max_attempts {
            self.enforce_rate_limit().await;
            let started_at = Instant::now();

            let response = self
                .client
                .post(&self.config.rpc_url)
                .json(&request_body)
                .send()
                .await;

            metrics()
                .last_rpc_latency_ms
                .store(started_at.elapsed().as_millis() as u64, Ordering::Relaxed);

            match response {
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();

                    if !status.is_success() {
                        let message = format!("RPC {method} HTTP {status}: {body}");
                        if should_retry_http_status(status)
                            && attempt + 1 < self.config.rpc_retry_policy.max_attempts
                        {
                            self.sleep_before_retry(method, attempt, &message).await;
                            continue;
                        }

                        return Err(anyhow!(message));
                    }

                    let payload: Value = serde_json::from_str(&body).with_context(|| {
                        format!("failed to decode RPC {method} response body: {body}")
                    })?;

                    if let Some(rpc_error) = payload.get("error") {
                        let message = rpc_error.to_string();
                        if should_retry_rpc_error(rpc_error)
                            && attempt + 1 < self.config.rpc_retry_policy.max_attempts
                        {
                            self.sleep_before_retry(method, attempt, &message).await;
                            continue;
                        }

                        return Err(anyhow!("RPC {method} error: {message}"));
                    }

                    return payload
                        .get("result")
                        .cloned()
                        .ok_or_else(|| anyhow!("missing result field in RPC {method} response"));
                }
                Err(err) => {
                    if attempt + 1 < self.config.rpc_retry_policy.max_attempts {
                        self.sleep_before_retry(method, attempt, &err.to_string())
                            .await;
                        continue;
                    }

                    return Err(anyhow!(err).context(format!("RPC request failed for {method}")));
                }
            }
        }

        Err(anyhow!("RPC request exhausted retries for method {method}"))
    }

    async fn enforce_rate_limit(&mut self) {
        if self.config.rpc_rate_limit_interval.is_zero() {
            self.last_request_started_at = Some(Instant::now());
            return;
        }

        if let Some(last_request_started_at) = self.last_request_started_at {
            let elapsed = last_request_started_at.elapsed();
            if elapsed < self.config.rpc_rate_limit_interval {
                tokio::time::sleep(self.config.rpc_rate_limit_interval - elapsed).await;
            }
        }

        self.last_request_started_at = Some(Instant::now());
    }

    async fn sleep_before_retry(&self, method: &str, attempt: u32, message: &str) {
        let delay = self.config.rpc_retry_policy.delay_for_attempt(attempt);
        metrics().total_rpc_retries.fetch_add(1, Ordering::Relaxed);

        warn!(
            method,
            attempt = attempt + 1,
            backoff_ms = delay.as_millis() as u64,
            error = message,
            "retrying RPC request",
        );

        tokio::time::sleep(delay).await;
    }
}

struct EventsResponse {
    latest_network_ledger: i64,
    events: Vec<Value>,
}

struct IndexerCycle {
    checkpoint: i64,
    latest_network_ledger: i64,
    inserted_events: u64,
}

impl IndexerCycle {
    fn caught_up(&self) -> bool {
        self.checkpoint >= self.latest_network_ledger
    }
}

pub async fn run_indexer_worker(pool: PgPool) {
    let config = IndexerConfig::from_env();
    let mut rpc = SorobanRpcClient::new(Client::new(), config.clone());
    let mut worker_retry_attempt = 0u32;

    info!(
        rpc_url = %config.rpc_url,
        idle_poll_ms = config.idle_poll_interval.as_millis() as u64,
        rpc_rate_limit_ms = config.rpc_rate_limit_interval.as_millis() as u64,
        rpc_retry_max_attempts = config.rpc_retry_policy.max_attempts,
        "starting Soroban indexer worker",
    );

    loop {
        let loop_started_at = Instant::now();

        match index_next_ledgers(&pool, &mut rpc).await {
            Ok(cycle) => {
                worker_retry_attempt = 0;

                let elapsed_ms = loop_started_at.elapsed().as_millis() as u64;
                let rate_per_second = if elapsed_ms == 0 {
                    cycle.inserted_events
                } else {
                    cycle.inserted_events.saturating_mul(1_000) / elapsed_ms.max(1)
                };

                metrics()
                    .last_loop_duration_ms
                    .store(elapsed_ms, Ordering::Relaxed);
                metrics()
                    .last_batch_events_processed
                    .store(cycle.inserted_events, Ordering::Relaxed);
                metrics()
                    .last_batch_rate_per_second
                    .store(rate_per_second, Ordering::Relaxed);

                if cycle.caught_up() {
                    debug!(
                        checkpoint = cycle.checkpoint,
                        latest_network_ledger = cycle.latest_network_ledger,
                        sleep_ms = config.idle_poll_interval.as_millis() as u64,
                        "indexer caught up; idling",
                    );
                    tokio::time::sleep(config.idle_poll_interval).await;
                }
            }
            Err(err) => {
                worker_retry_attempt = worker_retry_attempt.saturating_add(1);
                metrics().total_errors.fetch_add(1, Ordering::Relaxed);

                let backoff = config
                    .worker_retry_policy
                    .delay_for_attempt(worker_retry_attempt.saturating_sub(1));

                error!(
                    attempt = worker_retry_attempt,
                    backoff_ms = backoff.as_millis() as u64,
                    error = %err,
                    "indexer worker cycle failed",
                );

                tokio::time::sleep(backoff).await;
            }
        }
    }
}

async fn index_next_ledgers(pool: &PgPool, rpc: &mut SorobanRpcClient) -> Result<IndexerCycle> {
    let mut last_processed_ledger: i64 =
        sqlx::query_scalar("SELECT last_processed_ledger FROM indexer_state WHERE id = 1")
            .fetch_optional(pool)
            .await?
            .unwrap_or(0);

    if last_processed_ledger == 0 {
        let latest_network_ledger = rpc.get_latest_ledger().await?;

        sqlx::query(
            "INSERT INTO indexer_state (id, last_processed_ledger, updated_at)
             VALUES (1, $1, NOW())
             ON CONFLICT (id)
             DO UPDATE SET last_processed_ledger = EXCLUDED.last_processed_ledger, updated_at = NOW()",
        )
        .bind(latest_network_ledger)
        .execute(pool)
        .await?;

        metrics()
            .last_processed_ledger
            .store(latest_network_ledger, Ordering::Relaxed);

        info!(
            checkpoint = latest_network_ledger,
            "indexer initialized checkpoint from latest network ledger",
        );

        return Ok(IndexerCycle {
            checkpoint: latest_network_ledger,
            latest_network_ledger,
            inserted_events: 0,
        });
    }

    let start_ledger = last_processed_ledger + 1;
    let events_response = rpc.get_events(start_ledger).await?;

    if events_response.latest_network_ledger < start_ledger {
        metrics()
            .last_processed_ledger
            .store(last_processed_ledger, Ordering::Relaxed);

        return Ok(IndexerCycle {
            checkpoint: last_processed_ledger,
            latest_network_ledger: events_response.latest_network_ledger,
            inserted_events: 0,
        });
    }

    let mut transaction = pool.begin().await?;
    let mut inserted_events = 0u64;
    let mut max_seen_ledger = start_ledger.saturating_sub(1);

    for event in &events_response.events {
        let ledger = event
            .get("ledger")
            .and_then(parse_i64)
            .unwrap_or(start_ledger);
        let event_id = event.get("id").and_then(Value::as_str).unwrap_or_default();
        let contract_id = event
            .get("contractId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let topic_hash = event
            .get("topic")
            .and_then(Value::as_array)
            .and_then(|topics| topics.first())
            .and_then(Value::as_str)
            .unwrap_or_default();

        if event_id.is_empty() {
            warn!(ledger, "skipping event with empty id");
            continue;
        }

        max_seen_ledger = max_seen_ledger.max(ledger);

        let inserted = sqlx::query(
            "INSERT INTO indexed_events (id, ledger_amount, contract_id, topic_hash)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(event_id)
        .bind(ledger)
        .bind(contract_id)
        .bind(topic_hash)
        .execute(&mut *transaction)
        .await?;

        if inserted.rows_affected() == 0 {
            debug!(event_id, ledger, "skipping already-indexed event");
            continue;
        }

        inserted_events = inserted_events.saturating_add(1);
        metrics()
            .total_events_processed
            .fetch_add(1, Ordering::Relaxed);

        process_event_side_effects(&mut transaction, event)
            .await
            .with_context(|| format!("processing side effects for event {event_id}"))?;
    }

    let next_checkpoint = if max_seen_ledger >= start_ledger {
        max_seen_ledger
    } else {
        start_ledger
    };

    sqlx::query(
        "INSERT INTO indexer_state (id, last_processed_ledger, updated_at)
         VALUES (1, $1, NOW())
         ON CONFLICT (id)
         DO UPDATE SET last_processed_ledger = EXCLUDED.last_processed_ledger, updated_at = NOW()",
    )
    .bind(next_checkpoint)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    last_processed_ledger = next_checkpoint;
    metrics()
        .last_processed_ledger
        .store(last_processed_ledger, Ordering::Relaxed);

    info!(
        checkpoint = last_processed_ledger,
        latest_network_ledger = events_response.latest_network_ledger,
        inserted_events,
        "indexer cycle committed",
    );

    Ok(IndexerCycle {
        checkpoint: last_processed_ledger,
        latest_network_ledger: events_response.latest_network_ledger,
        inserted_events,
    })
}

async fn process_event_side_effects(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &Value,
) -> Result<()> {
    let topics = event.get("topic").and_then(Value::as_array);
    let first_topic = topics
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .unwrap_or("");

    match first_topic {
        "jobpost" | "jobauto" => {
            let job_id = topics
                .and_then(|items| items.get(1))
                .and_then(Value::as_str)
                .unwrap_or("0")
                .parse::<i64>()
                .unwrap_or(0);

            info!(job_id, "indexed job creation event");
        }
        "bid" => {
            info!("indexed bid submission event");
        }
        "accept" => {
            info!("indexed bid acceptance event");
        }
        "deposit" => {
            let sender = topics
                .and_then(|items| items.get(1))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let token = topics
                .and_then(|items| items.get(2))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let amount = event
                .get("value")
                .and_then(|value| value.get("xdr"))
                .and_then(Value::as_str)
                .map(|_| 0i64)
                .unwrap_or(0);
            let event_id = event.get("id").and_then(Value::as_str).unwrap_or_default();
            let ledger = event.get("ledger").and_then(parse_i64).unwrap_or(0);
            let contract_id = event
                .get("contractId")
                .and_then(Value::as_str)
                .unwrap_or_default();

            info!(
                event_id,
                ledger, contract_id, sender, token, amount, "indexed deposit event",
            );

            sqlx::query(
                "INSERT INTO deposits (id, ledger, contract_id, sender, amount, token)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(event_id)
            .bind(ledger)
            .bind(contract_id)
            .bind(sender)
            .bind(amount)
            .bind(token)
            .execute(&mut **tx)
            .await?;
        }
        _ => {}
    }

    Ok(())
}

fn read_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn read_env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn parse_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

fn should_retry_http_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn should_retry_rpc_error(error: &Value) -> bool {
    let message = error.to_string().to_lowercase();

    message.contains("rate limit")
        || message.contains("too many requests")
        || message.contains("temporar")
        || message.contains("timeout")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
    use sqlx::Row;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(rpc_url: String) -> IndexerConfig {
        IndexerConfig {
            rpc_url,
            idle_poll_interval: Duration::from_millis(1),
            rpc_rate_limit_interval: Duration::ZERO,
            rpc_retry_policy: RetryPolicy {
                max_attempts: 2,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(2),
            },
            worker_retry_policy: RetryPolicy {
                max_attempts: 2,
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(2),
            },
        }
    }

    #[test]
    fn retry_policy_caps_exponential_backoff() {
        let policy = RetryPolicy {
            max_attempts: 4,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(350),
        };

        assert_eq!(policy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(350));
        assert_eq!(policy.delay_for_attempt(6), Duration::from_millis(350));
    }

    #[tokio::test]
    async fn rpc_client_retries_rate_limited_requests() {
        let request_count = Arc::new(AtomicUsize::new(0));

        async fn rpc_handler(
            State(request_count): State<Arc<AtomicUsize>>,
        ) -> Result<Json<Value>, (StatusCode, String)> {
            let seen = request_count.fetch_add(1, AtomicOrdering::SeqCst);
            if seen == 0 {
                return Err((
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many requests".to_string(),
                ));
            }

            Ok(Json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": { "sequence": 12345 }
            })))
        }

        let app = Router::new()
            .route("/", post(rpc_handler))
            .with_state(request_count.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mut rpc =
            SorobanRpcClient::new(Client::new(), test_config(format!("http://{address}")));
        let latest_ledger = rpc.get_latest_ledger().await.unwrap();

        assert_eq!(latest_ledger, 12345);
        assert_eq!(request_count.load(AtomicOrdering::SeqCst), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn indexer_recovers_from_rpc_failure_and_resumes_from_checkpoint(pool: PgPool) {
        let mock_server = MockServer::start().await;

        sqlx::query("UPDATE indexer_state SET last_processed_ledger = $1 WHERE id = 1")
            .bind(41_i64)
            .execute(&pool)
            .await
            .unwrap();

        {
            let _guard = Mock::given(method("POST"))
                .and(path("/"))
                .respond_with(ResponseTemplate::new(500))
                .mount_as_scoped(&mock_server)
                .await;

            let mut rpc = SorobanRpcClient::new(Client::new(), test_config(mock_server.uri()));
            let result = index_next_ledgers(&pool, &mut rpc).await;
            assert!(result.is_err());
        }

        let checkpoint_after_failure: i64 =
            sqlx::query_scalar("SELECT last_processed_ledger FROM indexer_state WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(checkpoint_after_failure, 41);

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "latestLedger": 42,
                    "events": [
                        {
                            "id": "evt-42",
                            "ledger": "42",
                            "contractId": "CDUMMY",
                            "topic": ["deposit", "GABC123", "USDC"],
                            "value": { "xdr": "AAAA" }
                        }
                    ]
                }
            })))
            .mount(&mock_server)
            .await;

        let mut rpc = SorobanRpcClient::new(Client::new(), test_config(mock_server.uri()));
        let cycle = index_next_ledgers(&pool, &mut rpc).await.unwrap();

        assert_eq!(cycle.checkpoint, 42);
        assert_eq!(cycle.latest_network_ledger, 42);
        assert_eq!(cycle.inserted_events, 1);

        let checkpoint_after_recovery: i64 =
            sqlx::query_scalar("SELECT last_processed_ledger FROM indexer_state WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(checkpoint_after_recovery, 42);

        let indexed_event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM indexed_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(indexed_event_count, 1);

        let deposit_row = sqlx::query("SELECT sender, token FROM deposits WHERE id = 'evt-42'")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(deposit_row.get::<String, _>("sender"), "GABC123");
        assert_eq!(deposit_row.get::<String, _>("token"), "USDC");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn indexer_advances_empty_ledger_checkpoints_without_skipping(pool: PgPool) {
        let mock_server = MockServer::start().await;

        sqlx::query("UPDATE indexer_state SET last_processed_ledger = $1 WHERE id = 1")
            .bind(9_i64)
            .execute(&pool)
            .await
            .unwrap();

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "latestLedger": 11,
                    "events": []
                }
            })))
            .mount(&mock_server)
            .await;

        let mut rpc = SorobanRpcClient::new(Client::new(), test_config(mock_server.uri()));
        let cycle = index_next_ledgers(&pool, &mut rpc).await.unwrap();

        assert_eq!(cycle.checkpoint, 10);
        assert_eq!(cycle.latest_network_ledger, 11);
        assert_eq!(cycle.inserted_events, 0);
    }
}
