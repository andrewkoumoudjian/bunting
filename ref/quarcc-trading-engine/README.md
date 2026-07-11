## Install

**Prerequisites**
- C++ 23
- CMake ≥ 3.23

**Installing dependencies**

Arch:
```bash
sudo pacman -S websocketpp protobuf grpc
```

Build Server:
```bash
cmake -S . -B build
cmake --build build -j
```

Build Client:
```bash
pip install -r requirements.txt
bash ./scripts/generate_protos_python.py
```

## Usage

Server Terminal:

```bash
cd playground/
./build/engine-cpp/src/trading_engine
```

Client Terminal:

```bash
cd playground/python_client/
python3 client.py
```
