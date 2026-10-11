//! What the trader is looking at and typing, and how keys change it. Pure:
//! keys turn into [`Effect`]s (requests for the venue, quitting); the app
//! performs them.

use bunting_trading_state::bunting_client::FeedBook;
use bunting_trading_state::bunting_client::bnp_wire::{ClientMessage, Listing, Side};
use bunting_trading_state::{Instrument, Model, OrderRow, OrderTicket, TicketKind, TicketTif};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// What a key asks the app to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Send(ClientMessage),
    Quit,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tab {
    #[default]
    Orders,
    Fills,
    Account,
    Log,
}

impl Tab {
    pub const ALL: [Self; 4] = [Self::Orders, Self::Fills, Self::Account, Self::Log];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Orders => "Orders",
            Self::Fills => "Fills",
            Self::Account => "Account",
            Self::Log => "Log",
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::Orders => Self::Fills,
            Self::Fills => Self::Account,
            Self::Account => Self::Log,
            Self::Log => Self::Orders,
        }
    }
}

/// Ticket fields in tab order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    Venue,
    Side,
    Quantity,
    Kind,
    Price,
    Tif,
    PostOnly,
    Anonymous,
    Display,
}

impl Field {
    pub const ALL: [Self; 9] = [
        Self::Venue,
        Self::Side,
        Self::Quantity,
        Self::Kind,
        Self::Price,
        Self::Tif,
        Self::PostOnly,
        Self::Anonymous,
        Self::Display,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Venue => "Venue",
            Self::Side => "Side",
            Self::Quantity => "Quantity",
            Self::Kind => "Type",
            Self::Price => "Limit price",
            Self::Tif => "Time in force",
            Self::PostOnly => "Post only",
            Self::Anonymous => "Anonymous",
            Self::Display => "Display qty",
        }
    }

    fn step(self, forward: bool) -> Self {
        let index = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        let count = Self::ALL.len();
        Self::ALL[if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        }]
    }
}

/// An order being filled in. Numbers are kept as typed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketForm {
    pub ticket: OrderTicket,
    pub field: Field,
    pub quantity: String,
    pub price: String,
    pub display: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Ui {
    /// Index into the model's instruments.
    pub instrument: usize,
    /// Index into the selected instrument's venues.
    pub venue: usize,
    pub tab: Tab,
    /// Index into [`Ui::order_rows`].
    pub order: usize,
    pub ticket: Option<TicketForm>,
    pub confirm_kill: bool,
    pub help: bool,
}

impl Ui {
    #[must_use]
    pub fn instrument(&self, model: &Model) -> Option<Instrument> {
        let instruments = model.instruments();
        let index = self.instrument.min(instruments.len().saturating_sub(1));
        instruments.into_iter().nth(index)
    }

    /// The venue listing the trader has selected.
    #[must_use]
    pub fn listing(&self, model: &Model) -> Option<Listing> {
        let instrument = self.instrument(model)?;
        let index = self.venue.min(instrument.listings.len().saturating_sub(1));
        instrument.listings.get(index).map(|info| info.listing)
    }

    /// Orders as listed: live first, then the rest, newest first in each.
    #[must_use]
    pub fn order_rows(model: &Model) -> Vec<&OrderRow> {
        let mut rows: Vec<&OrderRow> = model.orders().iter().rev().collect();
        rows.sort_by_key(|row| !row.status.is_live());
        rows
    }

    /// Applies one key; returns what the app must do.
    pub fn handle(&mut self, key: KeyEvent, model: &mut Model, now_us: u64) -> Vec<Effect> {
        if key.kind == KeyEventKind::Release {
            return Vec::new();
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return vec![Effect::Quit];
        }
        if self.confirm_kill {
            self.confirm_kill = false;
            return if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                vec![Effect::Send(model.kill_switch(now_us))]
            } else {
                Vec::new()
            };
        }
        if self.help {
            self.help = false;
            return Vec::new();
        }
        if self.ticket.is_some() {
            return self.ticket_key(key, model, now_us);
        }
        match key.code {
            KeyCode::Char('q') => return vec![Effect::Quit],
            KeyCode::Char('?') => self.help = true,
            KeyCode::Up | KeyCode::Char('k') => self.instrument = self.instrument.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.instrument =
                    (self.instrument + 1).min(model.instruments().len().saturating_sub(1));
            }
            KeyCode::Left | KeyCode::Char('h') => self.venue = self.venue.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => {
                let venues = self.instrument(model).map_or(0, |i| i.listings.len());
                self.venue = (self.venue + 1).min(venues.saturating_sub(1));
            }
            KeyCode::Tab => self.tab = self.tab.next(),
            KeyCode::Char(digit @ '1'..='4') => {
                let index = usize::from(digit as u8 - b'1');
                self.tab = Tab::ALL[index];
            }
            KeyCode::PageUp | KeyCode::Char('[') => self.order = self.order.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(']') => {
                self.order = (self.order + 1).min(Self::order_rows(model).len().saturating_sub(1));
            }
            KeyCode::Char('b') => self.open_ticket(model, Side::Buy),
            KeyCode::Char('s') => self.open_ticket(model, Side::Sell),
            KeyCode::Char('c') => {
                let selected = Self::order_rows(model)
                    .get(self.order)
                    .map(|row| row.client_order_id);
                return selected
                    .and_then(|id| model.cancel(id))
                    .map(Effect::Send)
                    .into_iter()
                    .collect();
            }
            KeyCode::Char('C') => {
                return model.cancel_all().into_iter().map(Effect::Send).collect();
            }
            KeyCode::Char('K') => self.confirm_kill = true,
            KeyCode::Char('r') => {
                return model
                    .refresh(now_us)
                    .into_iter()
                    .map(Effect::Send)
                    .collect();
            }
            _ => {}
        }
        Vec::new()
    }

    /// Opens a ticket on the selected venue, priced to join the same side
    /// of that venue's book (or the other side when this one is empty).
    fn open_ticket(&mut self, model: &Model, side: Side) {
        let listing = self.listing(model);
        let book = listing
            .and_then(|listing| model.feed(listing))
            .map(|feed| &feed.book);
        let (same, other) = match side {
            Side::Buy => (
                book.and_then(FeedBook::best_bid),
                book.and_then(FeedBook::best_ask),
            ),
            Side::Sell => (
                book.and_then(FeedBook::best_ask),
                book.and_then(FeedBook::best_bid),
            ),
        };
        let price = same.or(other).map_or(0, |level| level.price);
        let mut ticket = OrderTicket::new(listing, side);
        ticket.price = price;
        self.ticket = Some(TicketForm {
            quantity: ticket.quantity.to_string(),
            price: if price > 0 {
                price.to_string()
            } else {
                String::new()
            },
            display: String::new(),
            ticket,
            field: Field::Quantity,
            error: None,
        });
    }

    fn ticket_key(&mut self, key: KeyEvent, model: &mut Model, now_us: u64) -> Vec<Effect> {
        let venues: Vec<Listing> = self
            .instrument(model)
            .map(|i| i.listings.iter().map(|info| info.listing).collect())
            .unwrap_or_default();
        let Some(form) = self.ticket.as_mut() else {
            return Vec::new();
        };
        form.error = None;
        match key.code {
            KeyCode::Esc => self.ticket = None,
            KeyCode::Tab | KeyCode::Down => form.field = form.field.step(true),
            KeyCode::BackTab | KeyCode::Up => form.field = form.field.step(false),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                let forward = key.code != KeyCode::Left;
                cycle(form, &venues, forward);
            }
            KeyCode::Backspace => {
                if let Some(text) = form.text_mut() {
                    text.pop();
                }
            }
            KeyCode::Char(digit @ '0'..='9') => {
                if let Some(text) = form.text_mut()
                    && text.len() < 18
                {
                    text.push(digit);
                }
            }
            KeyCode::Enter => {
                form.read_numbers();
                match model.place(&form.ticket, now_us) {
                    Ok(message) => {
                        // Remember the venue the trader chose.
                        if let Some(index) =
                            venues.iter().position(|&l| Some(l) == form.ticket.listing)
                        {
                            self.venue = index;
                        }
                        self.ticket = None;
                        self.tab = Tab::Orders;
                        self.order = 0;
                        return vec![Effect::Send(message)];
                    }
                    Err(error) => form.error = Some(error.to_string()),
                }
            }
            _ => {}
        }
        Vec::new()
    }
}

impl TicketForm {
    fn text_mut(&mut self) -> Option<&mut String> {
        match self.field {
            Field::Quantity => Some(&mut self.quantity),
            Field::Price => Some(&mut self.price),
            Field::Display => Some(&mut self.display),
            _ => None,
        }
    }

    fn read_numbers(&mut self) {
        self.ticket.quantity = self.quantity.parse().unwrap_or(0);
        self.ticket.price = self.price.parse().unwrap_or(0);
        self.ticket.display_quantity = if self.display.is_empty() {
            None
        } else {
            Some(self.display.parse().unwrap_or(0))
        };
    }
}

fn cycle(form: &mut TicketForm, venues: &[Listing], forward: bool) {
    let ticket = &mut form.ticket;
    match form.field {
        Field::Venue if !venues.is_empty() => {
            let index = venues
                .iter()
                .position(|&l| Some(l) == ticket.listing)
                .unwrap_or(0);
            let next = if forward {
                (index + 1) % venues.len()
            } else {
                (index + venues.len() - 1) % venues.len()
            };
            ticket.listing = Some(venues[next]);
        }
        Field::Side => {
            ticket.side = match ticket.side {
                Side::Buy => Side::Sell,
                Side::Sell => Side::Buy,
            };
        }
        Field::Kind => {
            ticket.kind = match ticket.kind {
                TicketKind::Limit => TicketKind::Market,
                TicketKind::Market => TicketKind::Limit,
            };
        }
        Field::Tif => {
            let index = TicketTif::ALL
                .iter()
                .position(|&t| t == ticket.tif)
                .unwrap_or(0);
            let count = TicketTif::ALL.len();
            ticket.tif = TicketTif::ALL[if forward {
                (index + 1) % count
            } else {
                (index + count - 1) % count
            }];
        }
        Field::PostOnly => ticket.post_only = !ticket.post_only,
        Field::Anonymous => ticket.anonymous = !ticket.anonymous,
        _ => {}
    }
}
