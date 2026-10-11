//! Command-line parsing and presentation over `bunting-client`.

use bunting_client::bnp_wire::{
    ClientMessage, FeedFlags, Listing, NegotiationInfo, NewOrder, NewsAudience, OperatorAction,
    OrderType, OtcDecision, ServerMessage, Side, TenderInfo, TimeInForce,
};
use bunting_client::{Client, ClientConfig, FeedBook, certificate_fingerprint};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(
    name = "bunting-trader",
    version,
    about = "Trade on a hosted Bunting venue over the Bunting Native Protocol"
)]
struct Cli {
    #[command(flatten)]
    connection: Connection,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Connection {
    /// The venue's BNP listener, `host:port`.
    #[arg(long, env = "BUNTING_SERVER", global = true)]
    server: Option<String>,
    /// The name on the venue's certificate (default: the host of --server).
    #[arg(long, env = "BUNTING_SERVER_NAME", global = true)]
    server_name: Option<String>,
    /// PEM file of the CA that issued the venue's certificate.
    #[arg(long, env = "BUNTING_CA", global = true)]
    ca: Option<PathBuf>,
    /// PEM file of your certificate (participant, instructor or administrator).
    #[arg(long, env = "BUNTING_CERT", global = true)]
    cert: Option<PathBuf>,
    /// PEM file of your certificate's private key.
    #[arg(long, env = "BUNTING_KEY", global = true)]
    key: Option<PathBuf>,
    /// Resume the private stream after this sequence (from a previous session).
    #[arg(long, global = true)]
    resume_after: Option<u64>,
}

#[derive(Subcommand)]
enum Command {
    /// Print a certificate's SHA-256 fingerprint for the venue roster.
    Fingerprint { certificate: PathBuf },
    /// List the venue's listings.
    Listings,
    /// Show one listing's visible book; `--watch` keeps it live.
    Book {
        venue: u128,
        instrument: u128,
        #[arg(long)]
        watch: bool,
    },
    /// Buy, then print the order's reports until it rests or completes.
    Buy(OrderArgs),
    /// Sell, then print the order's reports until it rests or completes.
    Sell(OrderArgs),
    /// Cancel an order by its client order ID.
    Cancel { client_order_id: u64 },
    /// Activate your kill switch: cancel all your live orders and refuse
    /// your new ones until you reset it. Other teams are unaffected.
    KillSwitch,
    /// Reset your kill switch so you can trade again.
    ResetKillSwitch,
    /// List your live orders.
    Orders,
    /// Show your cash and positions.
    Account,
    /// Print every private message (and optional feeds) until interrupted.
    Watch {
        /// Feeds to follow, as `venue:instrument`.
        #[arg(long = "feed", value_parser = parse_listing)]
        feeds: Vec<Listing>,
    },
    /// Read commands from standard input while printing events.
    Shell,
    /// Run the round (instructor and administrator certificates only).
    #[command(subcommand)]
    Operator(Operator),
    /// Accept or decline a tender offered to you.
    Tender {
        #[arg(value_enum)]
        decision: TenderChoice,
        tender: u128,
    },
    /// Negotiate an OTC trade with another participant.
    #[command(subcommand)]
    Otc(Otc),
    /// List your tenders and OTC negotiations.
    Deals,
}

#[derive(Clone, Copy, ValueEnum)]
enum TenderChoice {
    Accept,
    Decline,
}

#[derive(Clone, Copy, ValueEnum)]
enum Direction {
    Buy,
    Sell,
}

impl Direction {
    const fn side(self) -> Side {
        match self {
            Self::Buy => Side::Buy,
            Self::Sell => Side::Sell,
        }
    }
}

/// OTC negotiation steps (ADR 0043); each prints `DealDone` once committed.
#[derive(Subcommand)]
enum Otc {
    /// Propose a trade: you take `side`; the venue names the negotiation.
    Propose {
        counterparty: u128,
        instrument: u128,
        #[arg(value_enum)]
        side: Direction,
        quantity: i64,
        price: i64,
        /// Run time (nanoseconds) after which the proposal lapses.
        #[arg(long)]
        expires_ns: u64,
    },
    /// Counter a negotiation that awaits you.
    Counter {
        negotiation: u128,
        quantity: i64,
        price: i64,
    },
    /// Accept a negotiation that awaits you.
    Accept { negotiation: u128 },
    /// Reject a negotiation that awaits you.
    Reject { negotiation: u128 },
    /// Withdraw from an open negotiation.
    Break { negotiation: u128 },
}

impl Otc {
    fn message(self, request_id: u64) -> ClientMessage {
        let decide = |negotiation_id, decision| ClientMessage::OtcDecision {
            request_id,
            negotiation_id,
            decision,
        };
        match self {
            Self::Propose {
                counterparty,
                instrument,
                side,
                quantity,
                price,
                expires_ns,
            } => ClientMessage::OtcPropose {
                request_id,
                // Unique for you for the run, like the request ID.
                negotiation_id: request_id,
                counterparty_id: counterparty,
                instrument_id: instrument,
                side: side.side(),
                quantity,
                price,
                expires_at_ns: expires_ns,
            },
            Self::Counter {
                negotiation,
                quantity,
                price,
            } => ClientMessage::OtcCounter {
                request_id,
                negotiation_id: negotiation,
                quantity,
                price,
            },
            Self::Accept { negotiation } => decide(negotiation, OtcDecision::Accept),
            Self::Reject { negotiation } => decide(negotiation, OtcDecision::Reject),
            Self::Break { negotiation } => decide(negotiation, OtcDecision::Break),
        }
    }
}

/// Operator commands (ADR 0042); each prints `OperatorDone` once committed.
#[derive(Subcommand)]
enum Operator {
    /// Start the run.
    Start,
    /// Pause the run.
    Pause,
    /// Resume a paused run.
    Resume,
    /// Advance a stepped run.
    Advance { steps: u32 },
    /// End the run.
    Terminate { reason: String },
    /// Halt an instrument on every venue.
    Halt { instrument: u128, reason: String },
    /// Reopen a halted instrument.
    Unhalt { instrument: u128, reason: String },
    /// Cancel live orders, optionally one participant's or one instrument's.
    MassCancel {
        #[arg(long)]
        participant: Option<u128>,
        #[arg(long)]
        instrument: Option<u128>,
    },
    /// Publish news to everyone, or to one participant with --participant.
    News {
        id: u128,
        headline: String,
        #[arg(default_value = "")]
        body: String,
        #[arg(long)]
        participant: Option<u128>,
    },
    /// Freeze the iteration's scores.
    Score,
    /// Fine a participant, in the currency's minor units.
    Fine {
        participant: u128,
        currency: u128,
        amount: i128,
        reason: String,
    },
    /// Offer a participant a tender at `price` until run time `expires_ns`.
    OpenTender {
        id: u128,
        participant: u128,
        instrument: u128,
        #[arg(value_enum)]
        side: Direction,
        quantity: i64,
        price: i64,
        #[arg(long)]
        expires_ns: u64,
    },
}

impl Operator {
    fn action(self) -> OperatorAction {
        match self {
            Self::Start => OperatorAction::Start,
            Self::Pause => OperatorAction::Pause,
            Self::Resume => OperatorAction::Resume,
            Self::Advance { steps } => OperatorAction::Advance { steps },
            Self::Terminate { reason } => OperatorAction::Terminate { reason },
            Self::Halt { instrument, reason } => OperatorAction::Halt {
                instrument_id: instrument,
                halted: true,
                reason,
            },
            Self::Unhalt { instrument, reason } => OperatorAction::Halt {
                instrument_id: instrument,
                halted: false,
                reason,
            },
            Self::MassCancel {
                participant,
                instrument,
            } => OperatorAction::MassCancel {
                participant_id: participant,
                instrument_id: instrument,
            },
            Self::News {
                id,
                headline,
                body,
                participant,
            } => OperatorAction::PublishNews {
                news_id: id,
                audience: participant.map_or(NewsAudience::Public, NewsAudience::Participant),
                headline,
                body,
            },
            Self::Score => OperatorAction::Score,
            Self::Fine {
                participant,
                currency,
                amount,
                reason,
            } => OperatorAction::Fine {
                participant_id: participant,
                currency_id: currency,
                amount,
                reason,
            },
            Self::OpenTender {
                id,
                participant,
                instrument,
                side,
                quantity,
                price,
                expires_ns,
            } => OperatorAction::OpenTender {
                tender_id: id,
                participant_id: participant,
                instrument_id: instrument,
                side: side.side(),
                quantity,
                price,
                expires_at_ns: expires_ns,
            },
        }
    }
}

#[derive(Args, Clone)]
struct OrderArgs {
    venue: u128,
    instrument: u128,
    quantity: i64,
    /// Limit price in ticks; omit with --market.
    #[arg(long, required_unless_present = "market")]
    price: Option<i64>,
    /// A market order (immediate-or-cancel).
    #[arg(long, conflicts_with = "price")]
    market: bool,
    #[arg(long, value_enum, default_value_t = Tif::Gtc)]
    tif: Tif,
    /// Reject instead of trading on arrival.
    #[arg(long)]
    post_only: bool,
    /// Hide your broker identifier on venues that publish them.
    #[arg(long)]
    anonymous: bool,
    /// Iceberg display size.
    #[arg(long)]
    display: Option<i64>,
    /// Client order ID, unique for you for the whole run (default: now in µs).
    #[arg(long)]
    id: Option<u64>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Tif {
    Gtc,
    Ioc,
    Fok,
    Day,
}

fn parse_listing(value: &str) -> Result<Listing, String> {
    let (venue, instrument) = value
        .split_once(':')
        .ok_or_else(|| format!("expected venue:instrument, got {value}"))?;
    Ok(Listing {
        venue_id: venue
            .parse()
            .map_err(|_| format!("invalid venue {venue}"))?,
        instrument_id: instrument
            .parse()
            .map_err(|_| format!("invalid instrument {instrument}"))?,
    })
}

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

pub(crate) fn run() -> Result<(), String> {
    let cli = Cli::parse();
    if let Command::Fingerprint { certificate } = &cli.command {
        let pem = std::fs::read(certificate)
            .map_err(|error| format!("cannot read {}: {error}", certificate.display()))?;
        println!(
            "{}",
            certificate_fingerprint(&pem).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    let client = connect(&cli.connection)?;
    let welcome = client.welcome();
    eprintln!(
        "connected: run {} participant {} ({:?}), private stream {:?} at sequence {}",
        welcome.run_id, welcome.participant_id, welcome.role, welcome.resume, welcome.run_sequence
    );
    let mut ids = next_ids();
    let result = execute(&client, cli.command, &mut ids);
    eprintln!("resume cursor: {}", client.cursor());
    client.close();
    result
}

fn connect(connection: &Connection) -> Result<Client, String> {
    let missing = |name: &str| format!("--{name} (or BUNTING_{}) is required", name.to_uppercase());
    let server = connection.server.clone().ok_or_else(|| missing("server"))?;
    let server_name = connection.server_name.clone().unwrap_or_else(|| {
        server
            .rsplit_once(':')
            .map_or(server.as_str(), |(host, _)| host)
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned()
    });
    let mut config = ClientConfig::from_files(
        server,
        server_name,
        connection.ca.as_deref().ok_or_else(|| missing("ca"))?,
        connection.cert.as_deref().ok_or_else(|| missing("cert"))?,
        connection.key.as_deref().ok_or_else(|| missing("key"))?,
    )
    .map_err(|error| error.to_string())?;
    config.resume_after = connection.resume_after;
    concat!("bunting-trader/", env!("CARGO_PKG_VERSION")).clone_into(&mut config.client_name);
    Client::connect(&config).map_err(|error| error.to_string())
}

/// Client order and request IDs: microseconds since the Unix epoch, then
/// counting up, so manual sessions never reuse an ID within a run.
fn next_ids() -> impl FnMut() -> u64 {
    let mut next = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |elapsed| {
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
        });
    move || {
        next = next.saturating_add(1);
        next
    }
}

fn execute(client: &Client, command: Command, ids: &mut impl FnMut() -> u64) -> Result<(), String> {
    match command {
        Command::Fingerprint { .. } => Ok(()),
        Command::Listings => request(client, &ClientMessage::ListingsRequest { request_id: 1 }),
        Command::Book {
            venue,
            instrument,
            watch,
        } => book(
            client,
            Listing {
                venue_id: venue,
                instrument_id: instrument,
            },
            watch,
        ),
        Command::Buy(order) => place(client, Side::Buy, &order, ids()),
        Command::Sell(order) => place(client, Side::Sell, &order, ids()),
        Command::Cancel { client_order_id } => {
            send(client, &ClientMessage::CancelOrder { client_order_id })?;
            until(client, |message| match message {
                ServerMessage::OrderCanceled {
                    client_order_id: id,
                    ..
                }
                | ServerMessage::CancelRejected {
                    client_order_id: id,
                    ..
                } => *id == client_order_id,
                ServerMessage::Reject { reference, .. } => *reference == client_order_id,
                _ => false,
            })
        }
        Command::KillSwitch | Command::ResetKillSwitch => {
            let request_id = ids();
            let reset = matches!(command, Command::ResetKillSwitch);
            send(
                client,
                &if reset {
                    ClientMessage::KillSwitchReset { request_id }
                } else {
                    ClientMessage::KillSwitch { request_id }
                },
            )?;
            until(client, |message| match message {
                ServerMessage::KillSwitchActivated { .. } => !reset,
                ServerMessage::KillSwitchReset { .. } => reset,
                ServerMessage::Reject { reference, .. } => *reference == request_id,
                _ => false,
            })
        }
        Command::Orders => request(client, &ClientMessage::OpenOrdersRequest { request_id: 1 }),
        Command::Account => request(client, &ClientMessage::AccountRequest { request_id: 1 }),
        Command::Watch { feeds } => watch(client, &feeds),
        Command::Shell => shell(client, ids),
        // Command request IDs are unique for the sender for the run: a
        // reused one is a duplicate the venue does not apply again.
        Command::Operator(operator) => {
            let request_id = ids();
            send(
                client,
                &ClientMessage::OperatorCommand {
                    request_id,
                    action: operator.action(),
                },
            )?;
            until(client, |message| match message {
                ServerMessage::OperatorDone { request_id: id, .. }
                | ServerMessage::Reject { reference: id, .. } => *id == request_id,
                _ => false,
            })
        }
        Command::Tender { decision, tender } => {
            let request_id = ids();
            deal(
                client,
                request_id,
                &ClientMessage::TenderDecision {
                    request_id,
                    tender_id: tender,
                    accept: matches!(decision, TenderChoice::Accept),
                },
            )
        }
        Command::Otc(otc) => {
            let request_id = ids();
            deal(client, request_id, &otc.message(request_id))
        }
        Command::Deals => {
            send(client, &ClientMessage::DealsRequest { request_id: 1 })?;
            until(client, |message| {
                matches!(
                    message,
                    ServerMessage::Deals { .. } | ServerMessage::Reject { .. }
                )
            })
        }
    }
}

/// Sends a tender decision or OTC request and prints until it commits.
fn deal(client: &Client, request_id: u64, message: &ClientMessage) -> Result<(), String> {
    send(client, message)?;
    until(client, |message| match message {
        ServerMessage::DealDone { request_id: id, .. }
        | ServerMessage::Reject { reference: id, .. } => *id == request_id,
        _ => false,
    })
}

fn send(client: &Client, message: &ClientMessage) -> Result<(), String> {
    client.send(message).map_err(|error| error.to_string())
}

/// Prints messages until `done` matches one, or for at most [`WAIT`].
fn until(client: &Client, done: impl Fn(&ServerMessage) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if let Some(message) = client
            .recv_timeout(POLL)
            .map_err(|error| error.to_string())?
        {
            println!("{}", describe(&message));
            if done(&message) {
                return Ok(());
            }
        }
    }
    Err("no answer from the venue within 10 s".to_owned())
}

/// Sends a request with request ID 1 and prints its answer.
fn request(client: &Client, message: &ClientMessage) -> Result<(), String> {
    send(client, message)?;
    until(client, |message| {
        matches!(
            message,
            ServerMessage::Listings { .. }
                | ServerMessage::OpenOrders { .. }
                | ServerMessage::Account { .. }
                | ServerMessage::Reject { .. }
        )
    })
}

fn new_order(side: Side, order: &OrderArgs, client_order_id: u64) -> ClientMessage {
    ClientMessage::NewOrder(NewOrder {
        client_order_id: order.id.unwrap_or(client_order_id),
        listing: Listing {
            venue_id: order.venue,
            instrument_id: order.instrument,
        },
        side,
        quantity: order.quantity,
        order_type: match order.price {
            Some(price) if !order.market => OrderType::Limit { price },
            _ => OrderType::Market,
        },
        time_in_force: if order.market {
            TimeInForce::Ioc
        } else {
            match order.tif {
                Tif::Gtc => TimeInForce::Gtc,
                Tif::Ioc => TimeInForce::Ioc,
                Tif::Fok => TimeInForce::Fok,
                Tif::Day => TimeInForce::Day,
            }
        },
        post_only: order.post_only,
        anonymous: order.anonymous,
        display_quantity: order.display,
    })
}

fn place(client: &Client, side: Side, order: &OrderArgs, id: u64) -> Result<(), String> {
    let message = new_order(side, order, id);
    let ClientMessage::NewOrder(NewOrder {
        client_order_id, ..
    }) = message
    else {
        return Err("not an order".to_owned());
    };
    let sent = Instant::now();
    send(client, &message)?;
    until(client, |message| match message {
        ServerMessage::OrderRested {
            client_order_id: id,
            ..
        }
        | ServerMessage::OrderDone {
            client_order_id: id,
            ..
        }
        | ServerMessage::OrderCanceled {
            client_order_id: id,
            ..
        }
        | ServerMessage::OrderRejected {
            client_order_id: id,
            ..
        } => *id == client_order_id,
        ServerMessage::Reject { reference, .. } => *reference == client_order_id,
        _ => false,
    })?;
    eprintln!("round trip {} us", sent.elapsed().as_micros());
    Ok(())
}

fn book(client: &Client, listing: Listing, watch: bool) -> Result<(), String> {
    let mut feed = FeedBook::new(1);
    send(
        client,
        &ClientMessage::Subscribe {
            request_id: 1,
            listing,
            flags: FeedFlags::ALL,
        },
    )?;
    loop {
        let Some(message) = client
            .recv_timeout(POLL)
            .map_err(|error| error.to_string())?
        else {
            continue;
        };
        if let ServerMessage::Reject { reason, .. } = &message {
            return Err(reason.clone());
        }
        if feed.apply(&message).map_err(|error| error.to_string())? {
            print_book(&feed);
            if !watch {
                return Ok(());
            }
        } else {
            println!("{}", describe(&message));
        }
    }
}

fn print_book(feed: &FeedBook) {
    let (bids, asks) = (feed.bids(), feed.asks());
    println!(
        "{:>12} {:>10} | {:<10} {:<12}",
        "bid qty", "bid", "ask", "ask qty"
    );
    for index in 0..bids.len().max(asks.len()).min(10) {
        let bid = bids.get(index);
        let ask = asks.get(index);
        println!(
            "{:>12} {:>10} | {:<10} {:<12}",
            bid.map_or(String::new(), |level| level.quantity.to_string()),
            bid.map_or(String::new(), |level| level.price.to_string()),
            ask.map_or(String::new(), |level| level.price.to_string()),
            ask.map_or(String::new(), |level| level.quantity.to_string()),
        );
    }
    if let Some((price, quantity)) = feed.trades().last() {
        println!("last trade {quantity} @ {price}");
    }
}

fn watch(client: &Client, feeds: &[Listing]) -> Result<(), String> {
    let mut books = BTreeMap::new();
    for (index, listing) in feeds.iter().enumerate() {
        let request_id = u32::try_from(index + 1).map_err(|_| "too many feeds".to_owned())?;
        books.insert(request_id, FeedBook::new(request_id));
        send(
            client,
            &ClientMessage::Subscribe {
                request_id,
                listing: *listing,
                flags: FeedFlags::ALL,
            },
        )?;
    }
    loop {
        let Some(message) = client
            .recv_timeout(POLL)
            .map_err(|error| error.to_string())?
        else {
            continue;
        };
        println!("{}", describe(&message));
        for feed in books.values_mut() {
            if let Err(error) = feed.apply(&message) {
                eprintln!("feed error: {error}; restart to resubscribe");
            }
        }
    }
}

#[expect(clippy::too_many_lines, reason = "one arm per shell command")]
fn shell(client: &Client, ids: &mut impl FnMut() -> u64) -> Result<(), String> {
    eprintln!(
        "commands: buy|sell <venue> <instrument> <qty> <price|market>, cancel <id>, kill, reset, sub <venue> <instrument>, unsub <request>, orders, account, listings, ping, quit"
    );
    let (lines, input) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    let mut next_request = 1_u32;
    loop {
        if let Some(message) = client
            .recv_timeout(POLL)
            .map_err(|error| error.to_string())?
        {
            println!("{}", describe(&message));
        }
        let line = match input.try_recv() {
            Ok(line) => line,
            Err(mpsc::TryRecvError::Empty) => continue,
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
        };
        let words: Vec<&str> = line.split_whitespace().collect();
        let request = next_request;
        next_request = next_request.wrapping_add(1).max(1);
        let message = match words.as_slice() {
            [] => continue,
            ["quit" | "exit"] => return Ok(()),
            [side @ ("buy" | "sell"), venue, instrument, quantity, price] => {
                let parse = |value: &str| {
                    value
                        .parse::<i64>()
                        .map_err(|_| format!("not a number: {value}"))
                };
                let args = OrderArgs {
                    venue: venue.parse().map_err(|_| "invalid venue".to_owned())?,
                    instrument: instrument
                        .parse()
                        .map_err(|_| "invalid instrument".to_owned())?,
                    quantity: parse(quantity)?,
                    price: if *price == "market" {
                        None
                    } else {
                        Some(parse(price)?)
                    },
                    market: *price == "market",
                    tif: Tif::Gtc,
                    post_only: false,
                    anonymous: false,
                    display: None,
                    id: None,
                };
                let side = if *side == "buy" {
                    Side::Buy
                } else {
                    Side::Sell
                };
                let order = new_order(side, &args, ids());
                if let ClientMessage::NewOrder(order) = &order {
                    eprintln!("client order ID {}", order.client_order_id);
                }
                order
            }
            ["cancel", id] => ClientMessage::CancelOrder {
                client_order_id: id.parse().map_err(|_| "invalid order ID".to_owned())?,
            },
            ["kill"] => ClientMessage::KillSwitch {
                request_id: u64::from(request),
            },
            ["reset"] => ClientMessage::KillSwitchReset {
                request_id: u64::from(request),
            },
            ["sub", venue, instrument] => ClientMessage::Subscribe {
                request_id: request,
                listing: parse_listing(&format!("{venue}:{instrument}"))?,
                flags: FeedFlags::ALL,
            },
            ["unsub", id] => ClientMessage::Unsubscribe {
                request_id: id.parse().map_err(|_| "invalid request ID".to_owned())?,
            },
            ["orders"] => ClientMessage::OpenOrdersRequest {
                request_id: request,
            },
            ["account"] => ClientMessage::AccountRequest {
                request_id: request,
            },
            ["listings"] => ClientMessage::ListingsRequest {
                request_id: request,
            },
            ["ping"] => ClientMessage::Ping {
                nonce: u64::from(request),
            },
            _ => {
                eprintln!("unknown command: {line}");
                continue;
            }
        };
        send(client, &message)?;
    }
}

/// One line per server message.
#[expect(clippy::too_many_lines, reason = "one arm per server message")]
fn describe(message: &ServerMessage) -> String {
    let side = |side: &Side| match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    };
    match message {
        ServerMessage::OrderAccepted {
            stamp,
            client_order_id,
            listing,
            side: order_side,
            quantity,
            price,
            ..
        } => format!(
            "#{} accepted {client_order_id}: {} {quantity} {}:{} @ {}",
            stamp.sequence,
            side(order_side),
            listing.venue_id,
            listing.instrument_id,
            price.map_or("market".to_owned(), |price| price.to_string())
        ),
        ServerMessage::OrderRejected {
            stamp,
            client_order_id,
            reason,
        } => format!("#{} rejected {client_order_id}: {reason:?}", stamp.sequence),
        ServerMessage::OrderRested {
            stamp,
            client_order_id,
            price,
            remaining,
            reference,
            ..
        } => format!(
            "#{} resting {client_order_id}: {remaining} @ {price}{}",
            stamp.sequence,
            reference.map_or_else(String::new, |reference| format!(
                " (feed reference {reference})"
            ))
        ),
        ServerMessage::OrderReduced {
            stamp,
            client_order_id,
            remaining,
            ..
        } => format!("#{} {client_order_id} leaves {remaining}", stamp.sequence),
        ServerMessage::Fill {
            stamp,
            client_order_id,
            side: fill_side,
            price,
            quantity,
            fee,
            liquidity,
            ..
        } => format!(
            "#{} fill {client_order_id}: {} {quantity} @ {price} ({liquidity:?}, fee {fee})",
            stamp.sequence,
            side(fill_side)
        ),
        ServerMessage::OrderDone {
            stamp,
            client_order_id,
            ..
        } => format!("#{} done {client_order_id}", stamp.sequence),
        ServerMessage::OrderCanceled {
            stamp,
            client_order_id,
            remaining,
            reason,
            ..
        } => format!(
            "#{} canceled {client_order_id}: {remaining} left ({reason:?})",
            stamp.sequence
        ),
        ServerMessage::CancelRejected {
            stamp,
            client_order_id,
            reason,
        } => format!(
            "#{} cancel rejected {client_order_id}: {reason:?}",
            stamp.sequence
        ),
        ServerMessage::Reject {
            request_type,
            reference,
            reason,
        } => format!("refused request {request_type:#04x} {reference}: {reason}"),
        ServerMessage::Listings { listings, .. } => listings
            .iter()
            .map(|info| {
                format!(
                    "{}:{} {}",
                    info.listing.venue_id, info.listing.instrument_id, info.symbol
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ServerMessage::OpenOrders { orders, .. } if orders.is_empty() => {
            "no live orders".to_owned()
        }
        ServerMessage::OpenOrders { orders, .. } => orders
            .iter()
            .map(|order| {
                format!(
                    "{} {} {}/{} {}:{} @ {}",
                    order.client_order_id,
                    side(&order.side),
                    order.remaining_quantity,
                    order.original_quantity,
                    order.listing.venue_id,
                    order.listing.instrument_id,
                    order.price
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ServerMessage::Account {
            cash, positions, ..
        } => cash
            .iter()
            .map(|cash| {
                format!(
                    "cash {}: {} ({} reserved)",
                    cash.currency_id, cash.balance, cash.reserved
                )
            })
            .chain(positions.iter().map(|position| {
                format!(
                    "position {}: {} (open buy {}, open sell {})",
                    position.instrument_id,
                    position.position,
                    position.open_buy,
                    position.open_sell
                )
            }))
            .collect::<Vec<_>>()
            .join("\n"),
        ServerMessage::MarketUpdate {
            request_id,
            listing,
            first_report,
            entries,
        } => format!(
            "feed {request_id} {}:{} from report {first_report}: {} entries",
            listing.venue_id,
            listing.instrument_id,
            entries.len()
        ),
        ServerMessage::MarketSnapshot {
            request_id,
            bids,
            asks,
            ..
        } => format!(
            "snapshot {request_id}: {} bid levels, {} ask levels",
            bids.len(),
            asks.len()
        ),
        ServerMessage::OperatorDone { run_sequence, .. }
        | ServerMessage::DealDone { run_sequence, .. } => {
            format!("committed at run sequence {run_sequence}")
        }
        ServerMessage::Tender { stamp, tender } => {
            format!("#{} {}", stamp.sequence, describe_tender(tender))
        }
        ServerMessage::Negotiation { stamp, negotiation } => {
            format!("#{} {}", stamp.sequence, describe_negotiation(negotiation))
        }
        ServerMessage::Deals {
            tenders,
            negotiations,
            ..
        } if tenders.is_empty() && negotiations.is_empty() => {
            "no tenders or negotiations".to_owned()
        }
        ServerMessage::Deals {
            tenders,
            negotiations,
            ..
        } => tenders
            .iter()
            .map(describe_tender)
            .chain(negotiations.iter().map(describe_negotiation))
            .collect::<Vec<_>>()
            .join("\n"),
        ServerMessage::RunStatus { stamp, status } => {
            format!("#{} run {status}", stamp.sequence)
        }
        ServerMessage::ListingHalt {
            stamp,
            instrument_id,
            halted,
        } => format!(
            "#{} instrument {instrument_id} {}",
            stamp.sequence,
            if *halted { "halted" } else { "reopened" }
        ),
        ServerMessage::News {
            stamp,
            headline,
            body,
            ..
        } => format!("#{} news: {headline} {body}", stamp.sequence),
        other => format!("{other:?}"),
    }
}

fn describe_tender(tender: &TenderInfo) -> String {
    format!(
        "tender {} for participant {}: {:?} {} of {} @ {} until {} ns, {:?}",
        tender.tender_id,
        tender.participant_id,
        tender.side,
        tender.quantity,
        tender.instrument_id,
        tender.price,
        tender.expires_at_ns,
        tender.status
    )
}

fn describe_negotiation(negotiation: &NegotiationInfo) -> String {
    format!(
        "otc {}: {} {:?}s {} of {} @ {} to {}, {:?}, awaiting {}",
        negotiation.negotiation_id,
        negotiation.proposer_id,
        negotiation.side,
        negotiation.quantity,
        negotiation.instrument_id,
        negotiation.price,
        negotiation.counterparty_id,
        negotiation.status,
        negotiation.awaiting
    )
}
