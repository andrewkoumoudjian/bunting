#!/bin/bash
set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}Generating Python protobuf files...${NC}"

PROTO_DIR="contracts"
PYTHON_OUT="gen/python/contracts"

mkdir -p "$PYTHON_OUT"

# Generate
python -m grpc_tools.protoc \
  --proto_path="$PROTO_DIR" \
  --python_out="$PYTHON_OUT" \
  --grpc_python_out="$PYTHON_OUT" \
  "$PROTO_DIR"/*.proto

echo -e "${GREEN}✓ Generated Python protobuf files${NC}"

# Fix imports
echo -e "${BLUE}Fixing imports...${NC}"

for file in "$PYTHON_OUT"/*_pb2*.py; do
  if [[ -f "$file" ]]; then
    # Replace: import X_pb2 → from gen.python.contracts import X_pb2
    sed -i.bak 's/^import \(.*_pb2\) as \(.*\)$/from gen.python.contracts import \1 as \2/' "$file"
    rm "${file}.bak"
    echo "Fixed: $(basename $file)"
  fi
done

# Create __init__.py files
touch gen/__init__.py
touch gen/python/__init__.py
touch "$PYTHON_OUT/__init__.py"

echo -e "${GREEN}✓ Import paths fixed!${NC}"
echo -e "${GREEN}✓ Complete!${NC}"
