#pragma once

#include <trading/utils/order_id_types.h>

#include <atomic>
#include <chrono>
#include <format>

namespace quarcc {

// Is there really a point to generating ids for orders if the exchange/broker
// already gives us ids for them when created
class OrderIdGenerator {
public:
  OrderIdGenerator(const std::string &ord = "ORD") : id_specifier_(ord) {}

  LocalOrderId generate() {
    auto now = std::chrono::system_clock::now();
    auto timestamp = std::chrono::duration_cast<std::chrono::milliseconds>(
                         now.time_since_epoch())
                         .count();

    return std::format("{}_{}_{:06d}", id_specifier_, timestamp, counter_++);
  }

private:
  std::atomic<std::uint64_t> counter_{0};
  const std::string id_specifier_;
};

inline std::string get_current_time() noexcept {
  const auto now = std::chrono::system_clock::now();
  return std::format("{:%FT%TZ}", now);
}

} // namespace quarcc
