#pragma once

#include <cstdint>
#include <expected>
#include <string>

namespace quarcc {

// TODO: Add error types lmao
enum class ErrorType : std::uint8_t {
  Error,
  FailedOrder,
};

struct Error {
  std::string message_;
  ErrorType type_;
};

template <typename T> using Result = std::expected<T, Error>;

} // namespace quarcc
