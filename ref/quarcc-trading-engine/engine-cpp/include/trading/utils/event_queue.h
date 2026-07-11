#pragma once

#include <condition_variable>
#include <mutex>
#include <queue>
#include <stop_token>
#include <thread>

namespace quarcc {

// Helper for std::visit with multiple lambdas
// Usage: std::visit(overloaded{[](TypeA a){...}, [](TypeB b){...}}, variant)
template <class... Ts> struct overloaded : Ts... {
  using Ts::operator()...;
};

// Thread-safe, single-consumer blocking queue
template <typename T> class EventQueue {
public:
  void push(T event) {
    {
      std::lock_guard lk{mu_};
      q_.push(std::move(event));
      ++total_pushed_;
    }
    cv_.notify_one();
  }

  // Blocking pop
  // Returns false only when stop is requested AND queue is empty
  // Callers must call mark_processed() after handling the returned item
  bool pop(T &out, std::stop_token st) {
    std::unique_lock lk{mu_};
    cv_.wait(lk, st, [&] { return !q_.empty(); });
    if (q_.empty())
      return false;
    out = std::move(q_.front());
    q_.pop();
    return true;
  }

  // Called by the dispatch thread after each event is fully handled
  // Required for wait_idle() to be correct
  void mark_processed() {
    std::lock_guard lk{mu_};
    ++total_processed_;
    idle_cv_.notify_all();
  }

  // Block until all pushed events have been fully processed
  // Safe to call from any thread other than the dispatch thread
  void wait_idle() {
    std::unique_lock lk{mu_};
    idle_cv_.wait(lk, [&] { return total_processed_ == total_pushed_; });
  }

  std::size_t size() const {
    std::lock_guard lk{mu_};
    return q_.size();
  }

private:
  mutable std::mutex mu_;
  std::condition_variable_any cv_; // _any for stop_token overload in pop()
  std::condition_variable idle_cv_;
  std::queue<T> q_;
  uint64_t total_pushed_{0};
  uint64_t total_processed_{0};
};

} // namespace quarcc
