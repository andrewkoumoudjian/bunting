#include <iostream>
#include <trading/core/trading_engine.h>

int main(int argc, char **argv) {
  if (argc != 2) {
    std::cerr << "Invalid arguments. <config.yaml path>";
    return 1;
  }
  quarcc::TradingEngine engine;
  engine.Run(argv[1]);
}
