#pragma once

#include <chrono>
#include <cstdint>
#include <string>
#include <vector>

namespace quarcc {

enum class Event : std::uint8_t {
  ORDER_CREATED = 0,
  ORDER_VALIDATED = 1,
  ORDER_REJECTED = 2,
  ORDER_SUBMITTED = 3,
  ORDER_ACCEPTED = 4,
  ORDER_CANCELLED = 5,
  ORDER_REPLACED = 6,
  ORDER_EXPIRED = 7,

  KILL_SWITCH_ACTIVATED = 8,

  SYSTEM_STARTED = 9,
  SYSTEM_STOPPED = 10,
  GATEWAY_CONNECTED = 11,
  GATEWAY_DISCONNECTED = 12,
  ERROR_OCCURRED = 13,

  SIGNAL_RECEIVED = 14,
  SIGNAL_PROCESSED = 15,
  SIGNAL_IGNORED = 16,

  ORDER_FILLED = 17,
  ORDER_PARTIALLY_FILLED = 18
};

inline const char *event_to_string(Event event) {
  switch (event) {
  case Event::ORDER_CREATED:
    return "ORDER_CREATED";
  case Event::ORDER_VALIDATED:
    return "ORDER_VALIDATED";
  case Event::ORDER_REJECTED:
    return "ORDER_REJECTED";
  case Event::ORDER_SUBMITTED:
    return "ORDER_SUBMITTED";
  case Event::ORDER_ACCEPTED:
    return "ORDER_ACCEPTED";
  case Event::ORDER_CANCELLED:
    return "ORDER_CANCELLED";
  case Event::ORDER_REPLACED:
    return "ORDER_REPLACED";
  case Event::ORDER_EXPIRED:
    return "ORDER_EXPIRED";
  case Event::KILL_SWITCH_ACTIVATED:
    return "KILL_SWITCH_ACTIVATED";
  case Event::SYSTEM_STARTED:
    return "SYSTEM_STARTED";
  case Event::SYSTEM_STOPPED:
    return "SYSTEM_STOPPED";
  case Event::GATEWAY_CONNECTED:
    return "GATEWAY_CONNECTED";
  case Event::GATEWAY_DISCONNECTED:
    return "GATEWAY_DISCONNECTED";
  case Event::ERROR_OCCURRED:
    return "ERROR_OCCURRED";
  case Event::SIGNAL_RECEIVED:
    return "SIGNAL_RECEIVED";
  case Event::SIGNAL_PROCESSED:
    return "SIGNAL_PROCESSED";
  case Event::SIGNAL_IGNORED:
    return "SIGNAL_IGNORED";
  case Event::ORDER_FILLED:
    return "ORDER_FILLED";
  case Event::ORDER_PARTIALLY_FILLED:
    return "ORDER_PARTIALLY_FILLED";
  default:
    return "UNKNOWN";
  }
}

using Timestamp = std::chrono::system_clock::time_point;

struct LogEntry {
  std::uint64_t id;
  Timestamp timestamp;
  Event event_type;
  std::string data;
  std::string correlation_id;

  static Timestamp now() { return std::chrono::system_clock::now(); }

  static std::string timestamp_to_string(Timestamp ts) {
    auto time_t = std::chrono::system_clock::to_time_t(ts);
    std::tm tm;
    if (auto r = gmtime_r(&time_t, &tm); r == NULL) {
      throw std::runtime_error("gmtime_r error");
    }

    char buffer[32];
    std::strftime(buffer, sizeof(buffer), "%Y-%m-%d %H:%M:%S", &tm);

    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                  ts.time_since_epoch()) %
              1000;
    return std::format("{}.{:03d}", buffer, ms.count());
  }

  // TODO: More robust parser
  static Timestamp string_to_timestamp(const std::string &str) {
    std::tm tm = {};
    std::istringstream ss(str);
    ss >> std::get_time(&tm, "%Y-%m-%d %H:%M:%S");
    return std::chrono::system_clock::from_time_t(std::mktime(&tm));
  }
};

class IJournal {
public:
  virtual ~IJournal() = default;

  virtual void log(Event event, const std::string &data,
                   const std::string &correlation_id = "") = 0;

  virtual std::vector<LogEntry>
  get_history(Timestamp from, Timestamp to,
              std::optional<Event> event_filter = std::nullopt) = 0;

  virtual std::vector<LogEntry>
  get_order_history(const std::string &order_id) = 0;

  virtual void flush() = 0;
};

} // namespace quarcc
