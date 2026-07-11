#pragma once

#include <trading/core/order_manager.h>
#include <yaml-cpp/yaml.h>

#include <optional>
#include <string>
#include <vector>

namespace quarcc {

struct GrpcConfig {
  std::string host_post;
  std::string host;
  int port{};
};

struct NetworkConfig {
  GrpcConfig grpc;
};

struct DatabaseConfig {
  std::string journal = "trading_journal";
  std::string orders = "trading_orders";
};

struct MarketDataSubscription {
  Symbol symbol;
  BarPeriod period; // e.g. "1m", "5m", "1d"
};

struct MarketDataConfig {
  std::string feed; // "alpaca", "csv", "simulated"
  std::vector<MarketDataSubscription> subscriptions;
};

struct StrategyConfig {
  std::string id;
  std::string account_id;
  std::string gateway;
  DatabaseConfig database;
  // Optional: not all strategies need market data from the engine
  std::optional<MarketDataConfig> market_data;
};

struct Config {
  std::string account_id;
  NetworkConfig network;
  std::vector<StrategyConfig> strategies;
};

static inline DatabaseConfig parse_database(const std::string &id,
                                            const YAML::Node &dbNode) {
  DatabaseConfig db;

  if (!dbNode) {
    db.journal = std::format("{}_{}.db", id, db.journal);
    db.orders = std::format("{}_{}.db", id, db.orders);
    return db;
  }

  if (dbNode["journal"])
    db.journal =
        std::format("{}_{}.db", id, dbNode["journal"].as<std::string>());

  if (dbNode["orders"])
    db.orders = std::format("{}_{}.db", id, dbNode["orders"].as<std::string>());

  return db;
}

inline Config parse_config(const std::string &path) {
  YAML::Node root = YAML::LoadFile(path);

  Config cfg;

  cfg.network.grpc.host = root["network"]["grpc"]["host"].as<std::string>();
  cfg.network.grpc.port = root["network"]["grpc"]["port"].as<int>();
  cfg.network.grpc.host_post =
      std::format("{}:{}", cfg.network.grpc.host, cfg.network.grpc.port);

  for (const auto &node : root["strategies"]) {
    StrategyConfig s;
    s.id = node["id"].as<std::string>();
    s.account_id = node["account_id"].as<std::string>();
    s.gateway = node["gateway"].as<std::string>();
    s.database = parse_database(s.id, node["database"]);

    if (node["market_data"]) {
      MarketDataConfig md;
      md.feed = node["market_data"]["feed"].as<std::string>();

      for (const auto &sub : node["market_data"]["subscriptions"]) {
        md.subscriptions.push_back({
            sub["symbol"].as<std::string>(),
            sub["period"].as<std::string>(),
        });
      }

      s.market_data = std::move(md);
    }

    cfg.strategies.push_back(std::move(s));
  }

  return cfg;
}

} // namespace quarcc
