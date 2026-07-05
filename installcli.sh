#!/usr/bin/env bash
# Caddedit CLI-only Installer
# Installs just the `caddedit` CLI client - no server, no venv, no systemd.
# Use this on any machine you want to manage a Caddedit server FROM
# (e.g. your laptop), not the box actually running the service.

set -e

BASE_URL="https://chiuhuang.dev/caddedit"
DEFAULT_TARGET="/usr/local/bin/caddedit"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}=== Caddedit CLI Installer ===${NC}"

if ! command -v python3 &> /dev/null; then
  echo -e "${RED}Error: python3 is required (the CLI is a stdlib-only Python script).${NC}"
  exit 1
fi

if ! command -v curl &> /dev/null; then
  echo -e "${RED}Error: curl is required.${NC}"
  exit 1
fi

# Pick an install location: /usr/local/bin if we can write there (root or
# sudo), otherwise fall back to ~/.local/bin so this also works without sudo.
if [ -w "$(dirname "$DEFAULT_TARGET")" ] || [ "$EUID" -eq 0 ]; then
  TARGET="$DEFAULT_TARGET"
else
  mkdir -p "$HOME/.local/bin"
  TARGET="$HOME/.local/bin/caddedit"
  echo -e "${YELLOW}No write access to $(dirname "$DEFAULT_TARGET") - installing to $TARGET instead.${NC}"
  case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo -e "${YELLOW}Note: $HOME/.local/bin isn't on your PATH yet - add it to your shell profile.${NC}" ;;
  esac
fi

echo -e "\nDownloading caddedit CLI from $BASE_URL..."
curl -sSL "$BASE_URL/caddedit" -o "$TARGET"
chmod +x "$TARGET"
echo -e "${GREEN}Installed:${NC} $TARGET"

echo -e "\n${GREEN}=== Connect it to a server ===${NC}"
echo "If you have a connection string (from the web UI's 'CLI Connect' panel, or"
echo "from running 'caddedit config export' on another machine), paste it below."
echo "Otherwise just press enter and log in with the unlock password afterwards."
echo ""
read -p "Connection string (or press enter to skip): " CONN_STRING

if [ -n "$CONN_STRING" ]; then
  "$TARGET" config "$CONN_STRING"
else
  echo ""
  echo "Set it up manually with:"
  echo -e "  ${YELLOW}caddedit config set-url http://your-server:29048${NC}"
  echo -e "  ${YELLOW}caddedit config login${NC}              (enter the unlock password, get a token)"
  echo "Or, if you already have a token rather than the password:"
  echo -e "  ${YELLOW}caddedit config set-token${NC}"
fi

echo -e "\n${GREEN}Done.${NC} Try: ${YELLOW}caddedit list${NC}  (or just run 'caddedit' for the interactive menu)"