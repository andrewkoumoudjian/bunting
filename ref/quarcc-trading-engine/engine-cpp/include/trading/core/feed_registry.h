#pragma once

#include <trading/core/order_manager.h>
#include <trading/interfaces/i_market_data_feed.h>

#include <unordered_map>
#include <vector>

namespace quarcc {

using AccountId = std::string;
using FeedType = std::string;

struct FeedKey {
  FeedType feed_type;
  AccountId account_id;

  bool operator==(const FeedKey &) const = default;
};

struct FeedKeyHash {
  size_t operator()(const FeedKey &k) const {
    size_t h = std::hash<FeedType>{}(k.feed_type);
    h ^=
        std::hash<AccountId>{}(k.account_id) * 0x9e3779b9 + (h << 6) + (h >> 2);
    return h;
  }
};

// FeedRegistry basically handles:
//  - Different feeds (symbol & period)
//  - Subscription of the OM to the right feeds based on the config.yaml file
//  - Enqueues the bars/ticks to each OM's event queue
class FeedRegistry {
public:
  // Subscribes OM's to certain feeds, it's static right now but might think of
  // making it dynamic as well as for new strategies
  void register_subscription(FeedKey key, Symbol symbol, BarPeriod period,
                             OrderManager *om);

  void on_bar(const FeedKey &key, const Bar &bar);
  void on_tick(const FeedKey &key, const Tick &tick);

  // Start/Stops reading/streaming market data
  void start_all();
  void stop_all();

private:
  std::unique_ptr<IMarketDataFeed> create_feed(const FeedKey &key);

  // Bar subscriptions
  struct BarSubKey {
    FeedKey feed;
    Symbol symbol;
    BarPeriod period;
    bool operator==(const BarSubKey &) const = default;
  };

  struct BarSubKeyHash {
    size_t operator()(const BarSubKey &sk) const {
      size_t h = FeedKeyHash{}(sk.feed);
      h ^= std::hash<Symbol>{}(sk.symbol) * 0x9e3779b9 + (h << 6) + (h >> 2);
      h ^= std::hash<BarPeriod>{}(sk.period) * 0x9e3779b9 + (h << 6) + (h >> 2);
      return h;
    }
  };

  // Tick subscriptions
  struct TickSubKey {
    FeedKey feed;
    Symbol symbol;
    bool operator==(const TickSubKey &) const = default;
  };

  struct TickSubKeyHash {
    size_t operator()(const TickSubKey &sk) const {
      size_t h = FeedKeyHash{}(sk.feed);
      h ^= std::hash<Symbol>{}(sk.symbol) * 0x9e3779b9 + (h << 6) + (h >> 2);
      return h;
    }
  };

private:
  // Different types of feeds, "alpaca", "csv", "simulated" etc.
  // For actual external broker, I'm assuming each researcher will have it's
  // broker own account, so a websocket connection for each account will open
  std::unordered_map<FeedKey, std::unique_ptr<IMarketDataFeed>, FeedKeyHash>
      feeds_;

  // Subs to a specific bar
  std::unordered_map<BarSubKey, std::vector<OrderManager *>, BarSubKeyHash>
      bar_subs_;

  // Subs to a specific tick
  std::unordered_map<TickSubKey, std::vector<OrderManager *>, TickSubKeyHash>
      tick_subs_;
};

} // namespace quarcc
