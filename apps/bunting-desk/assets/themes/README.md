# Bundled desk themes

`bunting.json` holds the two themes that ship with the desk, in the
gpui-kit theme format (`ThemeSet`): **Bunting Dark** and **Bunting
Light**. Both carry the same 99 colour tokens; the key set follows
`gpui-component` 0.7.1's `src/theme/default-theme.json` (Longbridge
gpui-kit, Apache-2.0). The values are Bunting's own. Text and muted
text meet WCAG AA contrast on their backgrounds (4.5:1), as do the
boundaries that identify a control — `input.border`, `switch.background`
and the focus `ring` — at 3:1 (WCAG 1.4.11). Plain separators
(`border`, the sidebar/title/status borders, `table.row.border`) are
decorative and stay subtle by design.

Users can add their own themes as `*.json` files in the `themes` folder
of the desk's configuration directory; see `src/appearance.rs`.

## Tokens that carry market meaning

These tokens are not decoration. A theme that recolours them changes
what the desk says about the market, so keep the two sides distinct
(and distinguishable without relying on hue alone where you can):

| Token | Meaning |
|---|---|
| `chart.bullish` | Bid side, buy, price up |
| `chart.bearish` | Ask side, sell, price down |
| `success.background` | Connected, accepted, up (also read as the `success` text colour) |
| `danger.background` | Rejected, disconnected, kill switch, down (also read as the `danger` text colour) |
| `warning.background` | Halted or stale feed, needs attention |
| `info.background` | Neutral notice |
| `list.active.background` / `list.active.border` | Your own working order in a book or list |

The matching `*.foreground` tokens are the text colour to put *on* the
`*.background` colour (for example on a danger button), not a second
shade of the meaning itself.
