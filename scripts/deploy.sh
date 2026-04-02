#!/bin/bash
set -e

# Color definitions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# Determine the absolute path to the project root
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

echo -e "${BOLD}${CYAN}========================================${NC}"
echo -e "${BOLD}${CYAN}      Starting Deployment Pipeline      ${NC}"
echo -e "${BOLD}${CYAN}========================================${NC}"

# [1/5] Pre-check: Service Installation
SERVICE_NAME="barebones-reverse-proxy.service"
SERVICE_FILE="/etc/systemd/system/$SERVICE_NAME"
CONFIG_DIR="/etc/barebones-reverse-proxy"

if [ ! -f "$SERVICE_FILE" ]; then
    echo -e "${YELLOW}Service not found. Performing first-time installation...${NC}"
    
    if [[ $EUID -ne 0 ]]; then
       echo -e "${RED}Error: Initial installation requires root. Please run with sudo.${NC}"
       exit 1
    fi

    echo -e "${CYAN}Creating config directory: $CONFIG_DIR${NC}"
    mkdir -p "$CONFIG_DIR"
    
    if [ -f "proxy.conf" ]; then
        cp "proxy.conf" "$CONFIG_DIR/"
    fi
    
    if [ -d ".env" ]; then
        cp -r ".env" "$CONFIG_DIR/"
    fi

    echo -e "${CYAN}Installing systemd service unit...${NC}"
    cp "scripts/$SERVICE_NAME" "$SERVICE_FILE"
    systemctl daemon-reload
    systemctl enable "$SERVICE_NAME"
    echo -e "${GREEN}Installation complete.${NC}"
fi

# [2/5] Pull project changes
echo -e "${CYAN}[2/5] Pulling project updates...${NC}"
git pull origin main

# [3/5] Build project
echo -e "${CYAN}[3/5] Building production binary...${NC}"
cargo build --release

# [4/5] Update binary
echo -e "${CYAN}[4/5] Updating binary in /usr/local/bin...${NC}"
sudo cp target/release/barebones_reverse_proxy /usr/local/bin/barebones-reverse-proxy

# [5/5] Restart service
echo -e "${CYAN}[5/5] Restarting service...${NC}"
sudo systemctl restart "$SERVICE_NAME"

echo -e "${BOLD}${GREEN}========================================${NC}"
echo -e "${BOLD}${GREEN}         Deployment Complete!           ${NC}"
echo -e "${BOLD}${GREEN}========================================${NC}"
