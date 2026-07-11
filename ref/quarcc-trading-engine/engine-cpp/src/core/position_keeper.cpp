#include <trading/core/position_keeper.h>

namespace quarcc {

// Applies a broker fill to the in-memory position using signed quantity and
// a weighted-average entry price.
//
// Signed-qty convention: long > 0, short < 0.
//
// Avg-price rules:
//  - Opening from flat: avg = fill_price
//  - Adding to existing side: weighted average
//  - Reducing (but not flipping): avg unchanged
//  - Position flips sides: avg = fill_price of new side
//  - Goes flat: avg = 0
//  - fill_price == 0 (gateway didn't provide it): update qty only
void PositionKeeper::on_fill(const std::string &symbol, double fill_qty,
                             double fill_price, v1::Side side) {
  if (fill_qty <= 0.0)
    return;

  std::unique_lock lock(mutex_);
  auto &pos = positions_[symbol];
  pos.symbol = symbol;

  const double signed_fill = (side == v1::Side::BUY) ? fill_qty : -fill_qty;
  const double old_qty = pos.quantity;
  const double new_qty = old_qty + signed_fill;

  // realized PnL: only when the fill reduces/closes an existing position
  if (fill_price > 0.0 && old_qty != 0.0 && (old_qty * signed_fill < 0.0)) {
    const double closed_qty =
        std::min(std::abs(old_qty), std::abs(signed_fill));

    if (old_qty > 0.0) {
      // reducing a long with a sell
      pos.rPnL += (fill_price - pos.avgPrice) * closed_qty;
    } else {
      // reducing a short with a buy
      pos.rPnL += (pos.avgPrice - fill_price) * closed_qty;
    }
  }

  if (fill_price > 0.0) {
    if (new_qty == 0.0) {
      // position went flat
      pos.avgPrice = 0.0;
    } else if (old_qty == 0.0) {
      // opening a brand-new position
      pos.avgPrice = fill_price;
    } else if ((old_qty > 0.0 && new_qty < 0.0) ||
               (old_qty < 0.0 && new_qty > 0.0)) {
      // position flipped sides; the new position's cost basis is the fill
      pos.avgPrice = fill_price;
    } else if ((old_qty > 0.0 && signed_fill > 0.0) ||
               (old_qty < 0.0 && signed_fill < 0.0)) {
      // adding to the existing side: weighted average
      pos.avgPrice =
          (old_qty * pos.avgPrice + signed_fill * fill_price) / new_qty;
    }
    // else: reducing position without flipping — avg price is unchanged
  }

  pos.quantity = new_qty;
}

Result<v1::Position>
PositionKeeper::get_position(const std::string &symbol) const {
  std::shared_lock lock(mutex_);

  auto it = positions_.find(symbol);
  if (it == positions_.end()) [[unlikely]] {
    return std::unexpected(Error{"Position not found", ErrorType::Error});
  }

  v1::Position pos;
  pos.set_symbol(it->second.symbol);
  pos.set_quantity(it->second.quantity);
  pos.set_avg_price(it->second.avgPrice);
  pos.set_realized_pnl(it->second.rPnL);

  return pos;
}

v1::PositionList PositionKeeper::get_all_positions() const {
  std::shared_lock lock(mutex_);

  v1::PositionList all_pos;
  for (const auto &[symbol, curr_pos] : positions_) {
    v1::Position pos;
    pos.set_symbol(curr_pos.symbol);
    pos.set_quantity(curr_pos.quantity);
    pos.set_avg_price(curr_pos.avgPrice);
    pos.set_realized_pnl(curr_pos.rPnL);
    all_pos.add_positions()->CopyFrom(pos);
  }

  return all_pos;
}

}; // namespace quarcc
