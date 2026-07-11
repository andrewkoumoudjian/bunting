#pragma once

#include <chrono>
#include <condition_variable>
#include <functional>
#include <mutex>
#include <queue>
#include <thread>

namespace quarcc {

using Clock = std::chrono::steady_clock;
using TimePoint = Clock::time_point;
using PollFn = std::function<void()>;

struct PollTask {
  TimePoint next_run_;
  std::chrono::milliseconds interval_;
  std::string strategy_id_;
  PollFn fn_;

  bool operator>(const PollTask &o) const { return next_run_ > o.next_run_; }
};

class PollScheduler {
public:
  ~PollScheduler();

  void add_strategy(std::string id, std::chrono::milliseconds interval,
                    PollFn fn);
  void start();
  void stop();

private:
  void run();

private:
  std::priority_queue<PollTask, std::vector<PollTask>, std::greater<PollTask>>
      queue_;
  std::mutex mu_;
  std::condition_variable cv_;
  std::atomic<bool> running_{false};
  std::thread thread_;
};

} // namespace quarcc
