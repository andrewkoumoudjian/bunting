#pragma once

#include <mutex>
#include <optional>
#include <shared_mutex>
#include <stdexcept>
#include <string>
#include <unordered_map>

namespace quarcc {

using LocalOrderId = std::string;
using BrokerOrderId = std::string;

struct OrderId {
  LocalOrderId local_id;
  std::optional<BrokerOrderId> broker_id;

  explicit OrderId(LocalOrderId local)
      : local_id(std::move(local)), broker_id(std::nullopt) {}

  void set_broker_id(BrokerOrderId broker) { broker_id = std::move(broker); }

  std::string get_broker_id_or_throw() const {
    if (!broker_id) {
      throw std::runtime_error("Broker ID not yet assigned for local ID: " +
                               local_id);
    }
    return *broker_id;
  }

  std::string to_string() const {
    if (broker_id) {
      return local_id + " [broker: " + *broker_id + "]";
    }
    return local_id + " [pending]";
  }
};

class OrderIdMapper {
public:
  void add_mapping(const LocalOrderId &local, const BrokerOrderId &broker) {
    std::unique_lock lock(mutex_);
    local_to_broker_[local] = broker;
    broker_to_local_[broker] = local;
  }

  std::optional<BrokerOrderId> get_broker_id(const LocalOrderId &local) const {
    std::shared_lock lock(mutex_);
    auto it = local_to_broker_.find(local);
    return (it != local_to_broker_.end())
               ? std::optional<BrokerOrderId>(it->second)
               : std::nullopt;
  }

  std::optional<LocalOrderId> get_local_id(const BrokerOrderId &broker) const {
    std::shared_lock lock(mutex_);
    auto it = broker_to_local_.find(broker);
    return (it != broker_to_local_.end())
               ? std::optional<LocalOrderId>(it->second)
               : std::nullopt;
  }

  void remove_mapping(const LocalOrderId &local) {
    std::unique_lock lock(mutex_);
    auto it = local_to_broker_.find(local);
    if (it != local_to_broker_.end()) {
      broker_to_local_.erase(it->second);
      local_to_broker_.erase(it);
    }
  }

private:
  mutable std::shared_mutex mutex_;
  std::unordered_map<LocalOrderId, BrokerOrderId> local_to_broker_;
  std::unordered_map<BrokerOrderId, LocalOrderId> broker_to_local_;
};

} // namespace quarcc
