#include <print>
#include <trading/core/poll_scheduler.h>

namespace quarcc {

PollScheduler::~PollScheduler() { stop(); }

void PollScheduler::add_strategy(std::string id,
                                 std::chrono::milliseconds interval,
                                 PollFn fn) {
  std::lock_guard lk{mu_};
  queue_.push(PollTask{
      .next_run_ = Clock::now() + interval,
      .interval_ = interval,
      .strategy_id_ = std::move(id),
      .fn_ = std::move(fn),
  });
  cv_.notify_one();
}

void PollScheduler::start() {
  running_ = true;
  thread_ = std::thread(&PollScheduler::run, this);
}

void PollScheduler::stop() {
  running_ = false;
  cv_.notify_all();

  if (thread_.joinable())
    thread_.join();
}

void PollScheduler::run() {
  while (running_) {
    std::unique_lock lk{mu_};

    if (queue_.empty()) {
      cv_.wait(lk, [&] { return !queue_.empty() || !running_; });
      continue;
    }

    auto next = queue_.top().next_run_;
    cv_.wait_until(lk, next, [&] { return !running_; });
    if (!running_)
      break;

    if (Clock::now() < queue_.top().next_run_)
      continue;

    auto task = queue_.top();
    queue_.pop();
    lk.unlock();

    try {
      task.fn_();
    } catch (const std::exception &e) {
      std::println("[Scheduler] Strategy {} threw: {}", task.strategy_id_,
                   e.what());
    }

    task.next_run_ = Clock::now() + task.interval_;
    lk.lock();
    queue_.push(std::move(task));
  }
}

} // namespace quarcc
