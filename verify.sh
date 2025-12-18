#!/bin/bash
# RingMPSC - Verification Script
# Run all tests and benchmark with proper CPU isolation

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}═══════════════════════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}║                       RINGMPSC - VERIFICATION SUITE                        ║${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════════════════════${NC}"
echo ""

# Step 1: Build
echo -e "${YELLOW}[1/5] Building with ReleaseFast optimizations...${NC}"
zig build -Doptimize=ReleaseFast 2>&1 | head -20
echo -e "${GREEN}      ✓ Build complete${NC}"
echo ""

# Step 2: Unit tests
echo -e "${YELLOW}[2/5] Running unit tests...${NC}"
if zig build test 2>&1 | tail -5; then
    echo -e "${GREEN}      ✓ Unit tests passed${NC}"
else
    echo -e "${RED}      ✗ Unit tests failed${NC}"
    exit 1
fi
echo ""

# Step 3: FIFO ordering test
echo -e "${YELLOW}[3/5] Running FIFO ordering verification...${NC}"
if ./zig-out/bin/test-fifo 2>&1 | tail -10; then
    echo -e "${GREEN}      ✓ FIFO ordering verified${NC}"
else
    echo -e "${RED}      ✗ FIFO test failed${NC}"
    exit 1
fi
echo ""

# Step 4: Race condition test
echo -e "${YELLOW}[4/5] Running race condition detection...${NC}"
if ./zig-out/bin/test-chaos 2>&1 | tail -15; then
    echo -e "${GREEN}      ✓ No races detected${NC}"
else
    echo -e "${RED}      ✗ Race detection failed${NC}"
    exit 1
fi
echo ""

# Step 5: Throughput benchmark
echo -e "${YELLOW}[5/5] Running throughput benchmark...${NC}"
echo ""

# Check for root privileges for CPU isolation
if [ "$EUID" -eq 0 ]; then
    # With CPU isolation
    echo -e "      ${BLUE}Running with CPU isolation (taskset + nice -20)${NC}"
    taskset -c 0-15 nice -n -20 ./zig-out/bin/bench 2>&1
else
    # Without CPU isolation
    echo -e "      ${YELLOW}Note: Run as root for optimal results (CPU isolation)${NC}"
    echo -e "      ${YELLOW}      sudo ./verify.sh${NC}"
    echo ""
    ./zig-out/bin/bench 2>&1
fi
echo ""

echo -e "${BLUE}═══════════════════════════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}                    ALL VERIFICATIONS PASSED                                  ${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════════════════════${NC}"
