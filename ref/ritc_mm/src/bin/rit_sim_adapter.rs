use std::collections::{HashMap, VecDeque};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Form, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const DEFAULT_SIM_BASE_URL: &str = "http://localhost:8080/api";
const DEFAULT_SCENARIO_ID: &str = "normal_market";
const DEFAULT_STUDENT_ID: &str = "team_alpha";
const DEFAULT_TEAM_PASSWORD: &str = "secret123";
const DEFAULT_STEP_INTERVAL_MS: u64 = 100;
const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:9999";
const HISTORY_CAPACITY: usize = 4096;

#[derive(Clone)]
struct AppState {
    inner: Arc<RwLock<AdapterState>>,
    order_tx: mpsc::UnboundedSender<Value>,
}

struct AdapterState {
    config: AdapterConfig,
    run_id: String,
    status: String,
    tick: u32,
    ticks_per_period: u32,
    total_periods: u32,
    position: i64,
    avg_entry_price: f64,
    realized_pnl: f64,
    cash: f64,
    volume: i64,
    last_trade: f64,
    bid: f64,
    ask: f64,
    bid_size: i64,
    ask_size: i64,
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
    history: VecDeque<HistBar>,
    open_orders: HashMap<u64, AdapterOrder>,
    next_order_id: u64,
}

#[derive(Clone)]
struct AdapterConfig {
    sim_base_url: String,
    scenario_id: String,
    student_id: String,
    team_password: String,
    step_interval_ms: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct CaseResp {
    period: u32,
    tick: u32,
    ticks_per_period: u32,
    total_periods: u32,
    status: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct SecResp {
    ticker: String,
    position: i64,
    last: f64,
    bid: f64,
    bid_size: i64,
    ask: f64,
    ask_size: i64,
    volume: i64,
    is_tradeable: bool,
    limits: Vec<i64>,
}

#[derive(Clone, Deserialize, Serialize)]
struct BookLevel {
    price: f64,
    quantity: i64,
    quantity_filled: i64,
}

#[derive(Clone, Deserialize, Serialize)]
struct BookResp {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
}

#[derive(Clone, Deserialize, Serialize)]
struct OrderResp {
    order_id: u64,
    ticker: String,
    action: String,
    price: f64,
    quantity: i32,
    quantity_filled: i32,
    status: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct HistBar {
    tick: u32,
    close: f64,
}

#[derive(Clone, Deserialize, Serialize)]
struct PnlResp {
    position: i64,
    avg_entry_price: f64,
    realized_pnl: f64,
    unrealized_pnl: f64,
    total_pnl: f64,
    cash: f64,
    mark_price: f64,
}

#[derive(Clone)]
struct AdapterOrder {
    side: String,
    price: f64,
    quantity: i32,
    quantity_filled: i32,
    status: String,
}

#[derive(Deserialize)]
struct StartReplayResp {
    run_id: String,
    token: String,
    duration_sec: f64,
}

#[derive(Deserialize)]
struct MyRunsResp {
    runs: Vec<MyRunItem>,
}

#[derive(Deserialize)]
struct MyRunItem {
    run_id: String,
    completed_at: Option<String>,
}

#[derive(Deserialize)]
struct MarketEnvelope {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    step: u32,
    #[serde(default)]
    bid: f64,
    #[serde(default)]
    ask: f64,
    #[serde(default)]
    bid_size: i64,
    #[serde(default)]
    ask_size: i64,
    #[serde(default)]
    last_trade: Option<LastTradeField>,
    #[serde(default)]
    bids: Vec<LevelEntry>,
    #[serde(default)]
    asks: Vec<LevelEntry>,
    #[serde(default)]
    trades: Vec<PublicTrade>,
}

#[derive(Clone, Deserialize)]
struct LevelEntry {
    price: f64,
    qty: i64,
}

#[derive(Clone, Deserialize)]
struct PublicTrade {
    price: f64,
    qty: i64,
    #[serde(rename = "side", default)]
    _side: String,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum LastTradeField {
    Price(f64),
    Trade(PublicTrade),
}

#[derive(Deserialize)]
struct FillEnvelope {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    order_id: String,
    #[serde(default)]
    side: String,
    #[serde(default)]
    price: f64,
    #[serde(default)]
    qty: i32,
    #[serde(default)]
    remaining: i32,
    #[serde(rename = "is_maker", default)]
    _is_maker: bool,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct OrdersQuery {
    #[allow(dead_code)]
    status: Option<String>,
}

#[derive(Deserialize)]
struct BookQuery {
    #[allow(dead_code)]
    ticker: Option<String>,
    #[allow(dead_code)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct HistoryQuery {
    #[allow(dead_code)]
    ticker: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct NewOrderQuery {
    #[allow(dead_code)]
    ticker: Option<String>,
    action: String,
    price: f64,
    quantity: i32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AdapterConfig::from_env();
    let replay = start_replay(&config).await?;
    let ticks_per_period = ((replay.duration_sec * 1000.0) / config.step_interval_ms as f64) as u32;

    let state = AppState {
        inner: Arc::new(RwLock::new(AdapterState {
            config: config.clone(),
            run_id: replay.run_id.clone(),
            status: "ACTIVE".to_string(),
            tick: 0,
            ticks_per_period,
            total_periods: 1,
            position: 0,
            avg_entry_price: 0.0,
            realized_pnl: 0.0,
            cash: 0.0,
            volume: 0,
            last_trade: 0.0,
            bid: 0.0,
            ask: 0.0,
            bid_size: 0,
            ask_size: 0,
            bids: Vec::new(),
            asks: Vec::new(),
            history: VecDeque::with_capacity(HISTORY_CAPACITY),
            open_orders: HashMap::new(),
            next_order_id: 1,
        })),
        order_tx: mpsc::unbounded_channel::<Value>().0,
    };

    let order_tx =
        spawn_order_socket(state.clone(), &config, &replay.run_id, &replay.token).await?;
    let state = AppState {
        inner: state.inner.clone(),
        order_tx,
    };

    spawn_market_socket(state.clone()).await;
    spawn_stepper(state.clone());

    let app = Router::new()
        .route("/v1/case", get(get_case))
        .route("/v1/securities", get(get_securities))
        .route("/v1/securities/book", get(get_book))
        .route("/v1/securities/history", get(get_history))
        .route("/v1/pnl", get(get_pnl))
        .route("/v1/orders", get(get_orders).post(post_order))
        .route("/v1/orders/{id}", delete(cancel_order))
        .route("/v1/commands/cancel", post(cancel_all))
        .with_state(state);

    let addr: SocketAddr = env::var("RIT_ADAPTER_ADDR")
        .unwrap_or_else(|_| DEFAULT_SERVER_ADDR.to_string())
        .parse()?;

    println!(
        "adapter running on http://{} with run_id={} scenario={}",
        addr, replay.run_id, config.scenario_id
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

impl AdapterConfig {
    fn from_env() -> Self {
        Self {
            sim_base_url: env::var("SIM_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_SIM_BASE_URL.to_string()),
            scenario_id: env::var("SIM_SCENARIO_ID")
                .unwrap_or_else(|_| DEFAULT_SCENARIO_ID.to_string()),
            student_id: env::var("SIM_STUDENT_ID")
                .unwrap_or_else(|_| DEFAULT_STUDENT_ID.to_string()),
            team_password: env::var("SIM_TEAM_PASSWORD")
                .unwrap_or_else(|_| DEFAULT_TEAM_PASSWORD.to_string()),
            step_interval_ms: env::var("SIM_STEP_INTERVAL_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_STEP_INTERVAL_MS),
        }
    }
}

async fn start_replay(
    config: &AdapterConfig,
) -> Result<StartReplayResp, Box<dyn std::error::Error>> {
    let client = Client::builder().timeout(Duration::from_secs(10)).build()?;
    match try_start_replay(&client, config).await {
        Ok(replay) => Ok(replay),
        Err(error) if error.contains("Already running this scenario") => {
            stop_active_runs(&client, config).await?;
            Ok(try_start_replay(&client, config)
                .await
                .map_err(std::io::Error::other)?)
        }
        Err(error) => Err(Box::new(std::io::Error::other(error))),
    }
}

async fn try_start_replay(
    client: &Client,
    config: &AdapterConfig,
) -> Result<StartReplayResp, String> {
    let response = client
        .get(format!(
            "{}/replays/{}/start",
            config.sim_base_url, config.scenario_id
        ))
        .header("Authorization", format!("Bearer {}", config.student_id))
        .header("X-Team-Password", &config.team_password)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if response.status().is_success() {
        return response
            .json::<StartReplayResp>()
            .await
            .map_err(|error| error.to_string());
    }

    let body = response.text().await.map_err(|error| error.to_string())?;
    let parsed = serde_json::from_str::<Value>(&body).ok();
    Err(parsed
        .and_then(|json| {
            json.get("error")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or(body))
}

async fn stop_active_runs(
    client: &Client,
    config: &AdapterConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let runs = client
        .get(format!(
            "{}/replays/{}/my-runs",
            config.sim_base_url, config.scenario_id
        ))
        .header("Authorization", format!("Bearer {}", config.student_id))
        .send()
        .await?
        .error_for_status()?
        .json::<MyRunsResp>()
        .await?;

    for run in runs.runs {
        if run.completed_at.is_none() {
            client
                .delete(format!(
                    "{}/replays/{}/stop",
                    config.sim_base_url, run.run_id
                ))
                .header("Authorization", format!("Bearer {}", config.student_id))
                .send()
                .await?
                .error_for_status()?;
        }
    }
    Ok(())
}

async fn spawn_order_socket(
    state: AppState,
    config: &AdapterConfig,
    run_id: &str,
    token: &str,
) -> Result<mpsc::UnboundedSender<Value>, Box<dyn std::error::Error>> {
    let ws_url = format!(
        "{}/ws/orders?token={}&run_id={}",
        websocket_base_url(&config.sim_base_url),
        token,
        run_id
    );
    let (stream, _) = connect_async(ws_url).await?;
    let (mut writer, mut reader) = stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();

    tokio::spawn(async move {
        while let Some(value) = rx.recv().await {
            if writer.send(Message::Text(value.to_string())).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        while let Some(Ok(message)) = reader.next().await {
            if let Message::Text(text) = message {
                if let Ok(fill) = serde_json::from_str::<FillEnvelope>(&text) {
                    if fill.r#type == "FILL" {
                        apply_fill(&state, fill).await;
                        continue;
                    }
                }
                if let Ok(error) = serde_json::from_str::<ErrorEnvelope>(&text) {
                    if error.r#type == "ERROR" {
                        eprintln!("order-ws error: {}", error.message);
                    }
                }
            }
        }
    });

    Ok(tx)
}

async fn spawn_market_socket(state: AppState) {
    let (config, run_id) = {
        let guard = state.inner.read().await;
        (guard.config.clone(), guard.run_id.clone())
    };
    let ws_url = format!(
        "{}/ws/market?run_id={}",
        websocket_base_url(&config.sim_base_url),
        run_id
    );

    tokio::spawn(async move {
        let Ok((stream, _)) = connect_async(ws_url).await else {
            let mut guard = state.inner.write().await;
            guard.status = "STOPPED".to_string();
            return;
        };
        let (_, mut reader) = stream.split();

        while let Some(Ok(message)) = reader.next().await {
            if let Message::Text(text) = message {
                if let Ok(snapshot) = serde_json::from_str::<MarketEnvelope>(&text) {
                    if snapshot.r#type == "MARKET_DATA" {
                        let mut guard = state.inner.write().await;
                        apply_market_snapshot(&mut guard, snapshot);
                    }
                }
            }
        }

        let mut guard = state.inner.write().await;
        guard.status = "STOPPED".to_string();
    });
}

fn spawn_stepper(state: AppState) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis({
            let guard = state.inner.read().await;
            guard.config.step_interval_ms
        }));

        loop {
            ticker.tick().await;
            let status = {
                let guard = state.inner.read().await;
                guard.status.clone()
            };
            if status != "ACTIVE" {
                break;
            }
            let _ = state.order_tx.send(json!({ "action": "DONE" }));
        }
    });
}

fn apply_market_snapshot(state: &mut AdapterState, snapshot: MarketEnvelope) {
    let last_trade = snapshot.last_trade.as_ref().map(LastTradeField::price);
    let fallback_mid = if snapshot.bid > 0.0 && snapshot.ask > 0.0 {
        (snapshot.bid + snapshot.ask) / 2.0
    } else {
        0.0
    };
    let effective_close = match last_trade {
        Some(price) if price > 0.0 => price,
        _ => fallback_mid,
    };

    state.tick = snapshot.step;
    state.bid = snapshot.bid;
    state.ask = snapshot.ask;
    state.bid_size = snapshot.bid_size;
    state.ask_size = snapshot.ask_size;
    state.last_trade = effective_close;
    state.bids = snapshot
        .bids
        .into_iter()
        .map(|level| BookLevel {
            price: level.price,
            quantity: level.qty,
            quantity_filled: 0,
        })
        .collect();
    state.asks = snapshot
        .asks
        .into_iter()
        .map(|level| BookLevel {
            price: level.price,
            quantity: level.qty,
            quantity_filled: 0,
        })
        .collect();

    let trade_volume: i64 = snapshot.trades.iter().map(|trade| trade.qty).sum();
    state.volume += trade_volume;

    if effective_close > 0.0 {
        state.history.push_back(HistBar {
            tick: snapshot.step,
            close: effective_close,
        });
    }

    if state.tick >= state.ticks_per_period {
        state.status = "STOPPED".to_string();
    }

    while state.history.len() > HISTORY_CAPACITY {
        let _ = state.history.pop_front();
    }
}

impl LastTradeField {
    fn price(&self) -> f64 {
        match self {
            Self::Price(price) => *price,
            Self::Trade(trade) => trade.price,
        }
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn websocket_base_url(http_base: &str) -> String {
    if let Some(rest) = http_base.strip_prefix("https://") {
        format!("wss://{}", rest)
    } else if let Some(rest) = http_base.strip_prefix("http://") {
        format!("ws://{}", rest)
    } else {
        format!("ws://{}", http_base)
    }
}

async fn apply_fill(state: &AppState, fill: FillEnvelope) {
    let Ok(order_id) = fill.order_id.parse::<u64>() else {
        return;
    };

    let mut guard = state.inner.write().await;
    if let Some(order) = guard.open_orders.get_mut(&order_id) {
        order.quantity_filled += fill.qty;
        order.status = if fill.remaining > 0 {
            "PARTIAL".to_string()
        } else {
            "FILLED".to_string()
        };
    }

    let mut position = guard.position;
    let mut avg_entry_price = guard.avg_entry_price;
    let mut realized_pnl = guard.realized_pnl;
    let mut cash = guard.cash;
    apply_position_and_pnl(
        &mut position,
        &mut avg_entry_price,
        &mut realized_pnl,
        &mut cash,
        fill.side.as_str(),
        fill.price,
        i64::from(fill.qty),
    );
    guard.position = position;
    guard.avg_entry_price = avg_entry_price;
    guard.realized_pnl = realized_pnl;
    guard.cash = cash;
    guard.volume += i64::from(fill.qty);
    guard.last_trade = fill.price;
}

fn apply_position_and_pnl(
    position: &mut i64,
    avg_entry_price: &mut f64,
    realized_pnl: &mut f64,
    cash: &mut f64,
    side: &str,
    price: f64,
    qty: i64,
) {
    if qty <= 0 {
        return;
    }

    match side {
        "BUY" => {
            *cash -= price * qty as f64;
            if *position >= 0 {
                let current_qty = *position as f64;
                let new_qty = current_qty + qty as f64;
                *avg_entry_price = if new_qty > 0.0 {
                    (*avg_entry_price * current_qty + price * qty as f64) / new_qty
                } else {
                    0.0
                };
                *position += qty;
            } else {
                let cover_qty = qty.min(-*position);
                *realized_pnl += (*avg_entry_price - price) * cover_qty as f64;
                *position += cover_qty;
                let remainder = qty - cover_qty;
                if *position == 0 {
                    *avg_entry_price = 0.0;
                }
                if remainder > 0 {
                    *position = remainder;
                    *avg_entry_price = price;
                }
            }
        }
        "SELL" => {
            *cash += price * qty as f64;
            if *position <= 0 {
                let current_qty = (-*position) as f64;
                let new_qty = current_qty + qty as f64;
                *avg_entry_price = if new_qty > 0.0 {
                    (*avg_entry_price * current_qty + price * qty as f64) / new_qty
                } else {
                    0.0
                };
                *position -= qty;
            } else {
                let close_qty = qty.min(*position);
                *realized_pnl += (price - *avg_entry_price) * close_qty as f64;
                *position -= close_qty;
                let remainder = qty - close_qty;
                if *position == 0 {
                    *avg_entry_price = 0.0;
                }
                if remainder > 0 {
                    *position = -remainder;
                    *avg_entry_price = price;
                }
            }
        }
        _ => {}
    }
}

async fn get_case(State(state): State<AppState>) -> Json<CaseResp> {
    let guard = state.inner.read().await;
    Json(CaseResp {
        period: 1,
        tick: guard.tick,
        ticks_per_period: guard.ticks_per_period,
        total_periods: guard.total_periods,
        status: guard.status.clone(),
    })
}

async fn get_securities(State(state): State<AppState>) -> Json<Vec<SecResp>> {
    let guard = state.inner.read().await;
    Json(vec![SecResp {
        ticker: "ALGO".to_string(),
        position: guard.position,
        last: guard.last_trade,
        bid: guard.bid,
        bid_size: guard.bid_size,
        ask: guard.ask,
        ask_size: guard.ask_size,
        volume: guard.volume,
        is_tradeable: guard.status == "ACTIVE",
        limits: vec![-5000, 5000],
    }])
}

async fn get_book(State(state): State<AppState>, Query(query): Query<BookQuery>) -> Json<BookResp> {
    let guard = state.inner.read().await;
    let depth = query.limit.unwrap_or(20);
    Json(BookResp {
        bids: guard.bids.iter().take(depth).cloned().collect(),
        asks: guard.asks.iter().take(depth).cloned().collect(),
    })
}

async fn get_orders(
    State(state): State<AppState>,
    Query(_query): Query<OrdersQuery>,
) -> Json<Vec<OrderResp>> {
    let guard = state.inner.read().await;
    Json(
        guard
            .open_orders
            .iter()
            .filter(|(_, order)| matches!(order.status.as_str(), "OPEN" | "PARTIAL"))
            .map(|(order_id, order)| OrderResp {
                order_id: *order_id,
                ticker: "ALGO".to_string(),
                action: order.side.clone(),
                price: order.price,
                quantity: order.quantity,
                quantity_filled: order.quantity_filled,
                status: order.status.clone(),
            })
            .collect(),
    )
}

async fn get_history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> Json<Vec<HistBar>> {
    let guard = state.inner.read().await;
    let limit = query.limit.unwrap_or(300);
    let bars: Vec<HistBar> = guard.history.iter().rev().take(limit).cloned().collect();
    Json(bars.into_iter().rev().collect())
}

async fn get_pnl(State(state): State<AppState>) -> Json<PnlResp> {
    let guard = state.inner.read().await;
    let mark_price = if guard.bid > 0.0 && guard.ask > 0.0 {
        (guard.bid + guard.ask) / 2.0
    } else {
        guard.last_trade
    };
    let unrealized_pnl = match guard.position.cmp(&0) {
        std::cmp::Ordering::Greater => (mark_price - guard.avg_entry_price) * guard.position as f64,
        std::cmp::Ordering::Less => (guard.avg_entry_price - mark_price) * (-guard.position) as f64,
        std::cmp::Ordering::Equal => 0.0,
    };
    Json(PnlResp {
        position: guard.position,
        avg_entry_price: guard.avg_entry_price,
        realized_pnl: guard.realized_pnl,
        unrealized_pnl,
        total_pnl: guard.realized_pnl + unrealized_pnl,
        cash: guard.cash,
        mark_price,
    })
}

async fn post_order(
    State(state): State<AppState>,
    Form(order): Form<NewOrderQuery>,
) -> Result<Json<OrderResp>, (StatusCode, String)> {
    let (order_id, side) = {
        let mut guard = state.inner.write().await;
        let order_id = guard.next_order_id;
        guard.next_order_id += 1;
        let side = match order.action.as_str() {
            "BUY" => "BUY",
            "SELL" => "SELL",
            other => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("unsupported action {}", other),
                ))
            }
        }
        .to_string();

        guard.open_orders.insert(
            order_id,
            AdapterOrder {
                side: side.clone(),
                price: order.price,
                quantity: order.quantity,
                quantity_filled: 0,
                status: "OPEN".to_string(),
            },
        );
        (order_id, side)
    };

    let payload = json!({
        "action": "NEW",
        "order_id": order_id.to_string(),
        "side": side,
        "price": order.price,
        "qty": order.quantity,
    });
    state.order_tx.send(payload).map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "order socket unavailable".to_string(),
        )
    })?;

    let guard = state.inner.read().await;
    let saved = guard.open_orders.get(&order_id).expect("order inserted");
    Ok(Json(OrderResp {
        order_id,
        ticker: "ALGO".to_string(),
        action: saved.side.clone(),
        price: saved.price,
        quantity: saved.quantity,
        quantity_filled: saved.quantity_filled,
        status: saved.status.clone(),
    }))
}

async fn cancel_order(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, (StatusCode, String)> {
    {
        let mut guard = state.inner.write().await;
        if let Some(order) = guard.open_orders.get_mut(&id) {
            order.status = "CANCELLED".to_string();
        }
    }

    state
        .order_tx
        .send(json!({ "action": "CANCEL", "order_id": id.to_string() }))
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "order socket unavailable".to_string(),
            )
        })?;
    Ok(StatusCode::OK)
}

async fn cancel_all(State(state): State<AppState>) -> StatusCode {
    let ids: Vec<u64> = {
        let guard = state.inner.read().await;
        guard.open_orders.keys().copied().collect()
    };
    {
        let mut guard = state.inner.write().await;
        for id in &ids {
            if let Some(order) = guard.open_orders.get_mut(id) {
                order.status = "CANCELLED".to_string();
            }
        }
    }
    for id in ids {
        let _ = state
            .order_tx
            .send(json!({ "action": "CANCEL", "order_id": id.to_string() }));
    }
    StatusCode::OK
}
