#[derive(Clone)]
struct QuoteCandle {
    label: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Clone, Debug)]
pub struct TerminalSnapshot {
    pub profile: String,
    pub role: String,
    pub connection: String,
    pub sequence: String,
    pub status: SharedString,
    pub stale: bool,
    pub connected: bool,
}

pub struct Terminal {
    next_request_id: u128,
    active_preset: WorkspacePreset,
    market_order: bool,
    status: SharedString,
    client: Option<Box<FixClient>>,
    io: Option<IoTask>,
    _runtime: Option<Runtime>,
    command_input: Entity<InputState>,
    quantity_input: Entity<InputState>,
    price_input: Entity<InputState>,
    cancel_input: Entity<InputState>,
}

impl Terminal {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search symbol or run a command")
                .default_value("BNT")
        });
        let quantity_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Quantity")
                .default_value("10")
        });
        let price_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Price ticks")
                .default_value("100")
        });
        let cancel_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Order ID to cancel"));

        let (runtime, io, client, status) = match Self::connect_client() {
            Ok((runtime, io, client, profile)) => (
                Some(runtime),
                Some(io),
                Some(client),
                format!("Connecting to {profile}").into(),
            ),
            Err(error) => (None, None, None, format!("Client error: {error}").into()),
        };

        let mut terminal = Self {
            next_request_id: 1,
            active_preset: WorkspacePreset::Trading,
            market_order: false,
            status,
            client,
            io,
            _runtime: runtime,
            command_input,
            quantity_input,
            price_input,
            cancel_input,
        };
        terminal.schedule_poll(cx);
        terminal
    }

    pub fn command_input(&self) -> Entity<InputState> {
        self.command_input.clone()
    }

    pub fn active_preset(&self) -> WorkspacePreset {
        self.active_preset
    }

    pub fn set_active_preset(&mut self, preset: WorkspacePreset, cx: &mut Context<Self>) {
        self.active_preset = preset;
        self.status = format!("{} workspace loaded", preset.label()).into();
        cx.notify();
    }

    pub fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        self.client.as_ref().map_or_else(
            || TerminalSnapshot {
                profile: "local".to_owned(),
                role: "unverified".to_owned(),
                connection: "Disconnected".to_owned(),
                sequence: "-".to_owned(),
                status: self.status.clone(),
                stale: true,
                connected: false,
            },
            |client| TerminalSnapshot {
                profile: client.profile_name.clone(),
                role: client
                    .verified_role
                    .clone()
                    .unwrap_or_else(|| "unverified".to_owned()),
                connection: format!("{:?}", client.connection_state()),
                sequence: client.committed_sequence.clone(),
                status: self.status.clone(),
                stale: client.stale,
                connected: !client.stale,
            },
        )
    }

    fn connect_client() -> Result<(Runtime, IoTask, Box<FixClient>, String), String> {
        let config_path = env::var_os("BUNTING_TERMINAL_CONFIG").map(PathBuf::from);
        let (config, _) = TerminalConfig::load(config_path)?;
        let profile_name = env::var("BUNTING_TERMINAL_PROFILE")
            .unwrap_or_else(|_| config.selected_profile.clone());
        let mut profile = config.profile(&profile_name)?;
        if let Ok(endpoint) = env::var_os("BUNTING_TERMINAL_ENDPOINT") {
            if !endpoint.trim().is_empty() {
                profile.endpoint = endpoint;
            }
        }
        let credential_override = env::var("BUNTING_TERMINAL_PASSWORD").ok().or_else(|| {
            (profile_name == "local" && env::var_os(&profile.password_env).is_none())
                .then(|| "bunting-local-dev".to_owned())
        });

        let display_client = FixClient::new(
            profile_name.clone(),
            profile.clone(),
            credential_override.clone(),
        )
        .map_err(|error| error.to_string())?;
        let io_client = FixClient::new(profile_name.clone(), profile, credential_override)
            .map_err(|error| error.to_string())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("bunting-terminal-fix")
            .build()
            .map_err(|error| error.to_string())?;
        let io = {
            let _guard = runtime.enter();
            IoTask::spawn(io_client)
        };
        Ok((runtime, io, Box::new(display_client), profile_name))
    }

    fn schedule_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |terminal, cx| {
                    terminal.drain_io();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn drain_io(&mut self) {
        loop {
            let event = self.io.as_mut().and_then(|io| io.events.try_recv().ok());
            let Some(UiEvent::Snapshot {
                client,
                recovery_request,
                competition_request,
            }) = event
            else {
                break;
            };
            self.status = client.status.clone().into();
            self.client = Some(client);
            if recovery_request {
                let request = book_request(self.allocate_request_id());
                self.enqueue(OutboundCmd::Send(request));
            }
            if competition_request {
                let request_id = self.allocate_request_id();
                for request in competition_requests(request_id) {
                    self.enqueue(OutboundCmd::Send(request));
                }
            }
        }
    }

    fn allocate_request_id(&mut self) -> u128 {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        id
    }

    fn enqueue(&mut self, command: OutboundCmd) {
        let Some(io) = &self.io else {
            self.status = "FIX I/O is not available".into();
            return;
        };
        match io.outbound.try_send(command) {
            Ok(()) => self.status = "Command queued".into(),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                self.status = "FIX outbound queue full — command not sent".into();
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.status = "FIX I/O closed — command not sent".into();
            }
        }
    }

    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.enqueue(OutboundCmd::Reconnect);
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let request = book_request(self.allocate_request_id());
        self.enqueue(OutboundCmd::Send(request));
        let request_id = self.allocate_request_id();
        for request in competition_requests(request_id) {
            self.enqueue(OutboundCmd::Send(request));
        }
        cx.notify();
    }

    fn input_value(input: &Entity<InputState>, cx: &Context<Self>) -> String {
        input.read(cx).value().to_string()
    }

    pub fn submit_order(&mut self, side: &'static str, cx: &mut Context<Self>) {
        let quantity = match Self::input_value(&self.quantity_input, cx).parse::<i64>() {
            Ok(quantity) if quantity > 0 => quantity,
            _ => {
                self.status = "Order rejected locally: quantity must be positive".into();
                cx.notify();
                return;
            }
        };
        let price = if self.market_order {
            None
        } else {
            match Self::input_value(&self.price_input, cx).parse::<i64>() {
                Ok(price) if price > 0 => Some(price),
                _ => {
                    self.status = "Order rejected locally: limit price must be positive".into();
                    cx.notify();
                    return;
                }
            }
        };
        let request_id = self.allocate_request_id();
        self.enqueue(OutboundCmd::Send(new_order(
            request_id, side, quantity, price,
        )));
        self.status = format!(
            "{} {} QTY={} PRICE={}",
            if self.market_order { "MARKET" } else { "LIMIT" },
            side.to_ascii_uppercase(),
            quantity,
            price.map_or_else(|| "MKT".to_owned(), |value| value.to_string())
        )
        .into();
        cx.notify();
    }

    pub fn submit_at_best(&mut self, side: &'static str, cx: &mut Context<Self>) {
        let price = self.client.as_ref().and_then(|client| {
            if side == "buy" {
                client.book.asks.first().map(|level| level.0)
            } else {
                client.book.bids.first().map(|level| level.0)
            }
        });
        let Some(price) = price else {
            self.status = "No live best price".into();
            cx.notify();
            return;
        };
        let quantity = Self::input_value(&self.quantity_input, cx)
            .parse::<i64>()
            .ok()
            .filter(|quantity| *quantity > 0)
            .unwrap_or(1);
        let request_id = self.allocate_request_id();
        self.enqueue(OutboundCmd::Send(new_order(
            request_id,
            side,
            quantity,
            Some(price),
        )));
        self.status = format!(
            "Quick {} {} @ {}",
            side.to_ascii_uppercase(),
            quantity,
            price
        )
        .into();
        cx.notify();
    }

    pub fn cancel_order(&mut self, cx: &mut Context<Self>) {
        let order_id = match Self::input_value(&self.cancel_input, cx).parse::<u128>() {
            Ok(order_id) => order_id,
            Err(_) => {
                self.status = "Cancel rejected locally: invalid order ID".into();
                cx.notify();
                return;
            }
        };
        let replacement_id = self.allocate_request_id();
        self.enqueue(OutboundCmd::Send(cancel(order_id, replacement_id)));
        self.status = format!("Cancel requested for order {order_id}").into();
        cx.notify();
    }

    pub fn tender_action(
        &mut self,
        tender_id: u128,
        action: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Ok(tender_id) = u64::try_from(tender_id) else {
            self.status = "Tender identifier exceeds FIX action range".into();
            cx.notify();
            return;
        };
        self.enqueue(OutboundCmd::Send(competition_action(
            "U6",
            action,
            Some(tender_id),
            None,
        )));
        self.status = format!("Tender {tender_id} {action}").into();
        cx.notify();
    }

    pub fn run_action(&mut self, action: &'static str, cx: &mut Context<Self>) {
        self.enqueue(OutboundCmd::Send(competition_action(
            "UA", action, None, None,
        )));
        self.status = format!("Run action {action}").into();
        cx.notify();
    }

    pub fn set_market_order(&mut self, market_order: bool, cx: &mut Context<Self>) {
        self.market_order = market_order;
        cx.notify();
    }

    fn quote_candles(&self) -> Vec<QuoteCandle> {
        let Some(client) = &self.client else {
            return Vec::new();
        };
        let samples = client.prices.iter().rev().take(72).collect::<Vec<_>>();
        let mut previous = None;
        samples
            .into_iter()
            .rev()
            .enumerate()
            .map(|(index, sample)| {
                let close = (sample.bid as f64 + sample.ask as f64) / 2.0;
                let open = previous.replace(close).unwrap_or(close);
                QuoteCandle {
                    label: index.to_string(),
                    open,
                    high: sample.ask as f64,
                    low: sample.bid as f64,
                    close,
                }
            })
            .collect()
    }
}

