# Bunting trading terminal

A terminal trading app for Bunting venues over the Bunting Native Protocol
(ADR 0047). It shows every venue's book for the instrument you select side
by side (price levels, or every order with its place in the queue), the
instrument's consolidated tape, the trades as they reach you, your orders,
fills and account, and your connection with its measured delays. You choose the venue for
each order; each venue sits at its own distance, so a far venue answers
later and its book is older by the time you see it.

## Practise on your own machine

```bash
bunting tui --local
```

This starts a private venue (the same `bunting server` a host runs) with
fresh certificates in `$XDG_STATE_HOME/bunting/local-venue`, and connects to
it. Built-in agents quote BNT on venue 1; QRC is listed on venue 1 and on
venue 2, two milliseconds away. Use `--venue-dir` to keep the files
elsewhere and `--server-bin` to run another `bunting` or `bunting-server`.

## Connect to a hosted venue

Use the certificate the organizers registered for your team, with the same
flags and environment as `bunting-trader`:

```bash
bunting tui --server venue.example.com:9443 \
  --ca competition-ca.pem --cert team.pem --key team.key
```

Or save the venue in `$XDG_CONFIG_HOME/bunting/terminal.json` (paths are
relative to the file) and run `bunting tui --profile finals`:

```json
{
  "version": 2,
  "default": "finals",
  "profiles": {
    "finals": {
      "server": "venue.example.com:9443",
      "ca": "competition-ca.pem",
      "cert": "team.pem",
      "key": "team.key"
    }
  }
}
```

The terminal reconnects by itself and resumes your private reports from
where it left off.

## Keys

| Key | Action |
|---|---|
| `↑` `↓` | instrument |
| `←` `→` | venue |
| `b` / `s` | buy / sell ticket on the selected venue |
| `Tab`, `←` `→`, digits, `Enter` | in the ticket: field, change, edit, send |
| `o` | books: price levels or every order with its queue place |
| `[` `]` | select an order |
| `c` / `C` | cancel the selected order / all your orders |
| `K` | kill switch (asks first); when active, reset it |
| `r` | reload open orders and account |
| `1`–`4` | orders, fills, account, log |
| `?` / `q` | help / quit |

## What the numbers mean

- **Order rt**: time from sending an order to the venue's first answer,
  measured here: your network both ways plus the venue's distance both ways.
- **Ping**: round trip to the venue host with no virtual distance.
- **Venue measures**: the one-way delay the venue measured for your
  connection. It is published to you and never used to order anything.
- **Queue** (press `o`): each displayed order's place at its price and its
  broker, on venues that publish brokers. Your own orders are not marked
  in this view yet; `mine` in the levels view is your resting quantity.
- **Tape from the hub**: every venue's best bid and offer (bold: the best
  across venues) and every venue's trades, published by the processor at
  the hub. Near a venue it lags that venue's direct feed.
- **Direct feeds, as they reached you**: every venue's trades in arrival
  order, with broker identifiers where the venue publishes them.
