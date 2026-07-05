#!/usr/bin/env bash
# Caddedit Installation Script
# Hosts project files and runs setup on the target server.

set -e

# Configuration
BASE_URL="https://chiuhuang.dev/caddedit"
INSTALL_DIR="/opt/caddedit"
CADDYFILE="/etc/caddy/Caddyfile"
VHOSTS_DIR="/etc/caddy/vhosts"
DEFAULT_PORT="29048"
CLI_BIN="/usr/local/bin/caddedit"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}=== Caddedit Installation Script ===${NC}"

# 1. Root privilege check
if [ "$EUID" -ne 0 ]; then
  echo -e "${RED}Error: Please run this script as root (sudo).${NC}"
  exit 1
fi

# 2. Check for dependencies
echo -e "\nChecking system dependencies..."
for cmd in curl python3; do
  if ! command -v "$cmd" &> /dev/null; then
    echo -e "${RED}Error: Required command '$cmd' is not installed. Please install it first.${NC}"
    exit 1
  fi
done

# Check python3 version
PYTHON_VER=$(python3 -c 'import sys; print(".".join(map(str, sys.version_info[:2])))')
echo "Python version: $PYTHON_VER"

# Check for uv (faster) or pip/venv
HAS_UV=false
if command -v uv &> /dev/null; then
  HAS_UV=true
  echo "Found 'uv' package manager (will use for faster setup)."
fi

# 3. Create installation directories
echo -e "\nCreating installation directories in $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR/templates"

# 4. Download Caddedit files from BASE_URL
echo -e "\nDownloading Caddedit files from $BASE_URL..."
curl -sSL "$BASE_URL/pyproject.toml" -o "$INSTALL_DIR/pyproject.toml"
curl -sSL "$BASE_URL/manager.py" -o "$INSTALL_DIR/manager.py"
curl -sSL "$BASE_URL/templates/login.html" -o "$INSTALL_DIR/templates/login.html"
curl -sSL "$BASE_URL/templates/index.html" -o "$INSTALL_DIR/templates/index.html"

# 5. Set up Python Virtual Environment and Install Dependencies
echo -e "\nSetting up Python environment..."
cd "$INSTALL_DIR"

if [ "$HAS_UV" = true ]; then
  uv venv .venv
  # Install dependencies from pyproject.toml
  uv pip install fastapi uvicorn python-dotenv httpx pydantic
else
  # Fallback to standard python3 venv
  python3 -m venv .venv
  .venv/bin/pip install --upgrade pip
  .venv/bin/pip install fastapi uvicorn python-dotenv httpx pydantic
fi

# 6. Configuration Prompts
echo -e "\n${GREEN}=== Configuring Caddedit ===${NC}"

# CADDEDIT_PASSWORD
read -p "Enter UI Unlock Password (press enter to generate a secure random one): " PASSWORD
if [ -z "$PASSWORD" ]; then
  PASSWORD=$(python3 -c 'import secrets; print(secrets.token_urlsafe(12))')
  echo -e "Generated secure password: ${YELLOW}$PASSWORD${NC}"
fi

# Disable AI prompt
read -p "Disable all AI Fallback features? [Y/n]: " DISABLE_AI_INPUT
DISABLE_AI_INPUT=${DISABLE_AI_INPUT:-Y}
if [[ "$DISABLE_AI_INPUT" =~ ^[Yy]$ ]]; then
  DISABLE_AI="true"
  COHERE_API_KEY=""
else
  DISABLE_AI="false"
  read -p "Enter Cohere API Key (optional): " COHERE_API_KEY
fi

# Port selection
read -p "Enter port to run Caddedit on [default: $DEFAULT_PORT]: " PORT
PORT=${PORT:-$DEFAULT_PORT}

# Write environment configuration to .env
echo -e "\nWriting configuration to .env..."
cat << ENVEOF > "$INSTALL_DIR/.env"
CADDEDIT_PASSWORD=$PASSWORD
CADDYFILE_PATH=$CADDYFILE
VHOSTS_DIR=$VHOSTS_DIR
CADDY_BACKUP_DIR=/etc/caddy/txg1-router-backups
DISABLE_AI=$DISABLE_AI
COHERE_API_KEY=$COHERE_API_KEY
COHERE_MODEL=command-a-03-2025
CADDY_RELOAD_COMMAND=caddy reload --config $CADDYFILE
HARDCODED_RULES_PATH=$INSTALL_DIR/hardcoded-rules.json
PORT=$PORT
HOST=0.0.0.0
ENVEOF
chmod 600 "$INSTALL_DIR/.env"

# 7. Vhost migration / Auto move vhosts
echo -e "\n${GREEN}=== Caddyfile Site Blocks Migration ===${NC}"
read -p "Do you want to automatically migrate existing monolithic Caddyfile site blocks into individual split vhost files? [Y/n]: " MIGRATE_INPUT
MIGRATE_INPUT=${MIGRATE_INPUT:-Y}

if [[ "$MIGRATE_INPUT" =~ ^[Yy]$ ]]; then
  echo "Running migration script..."

  # Run migration programmatically via Python
  .venv/bin/python3 -c "
import sys
from pathlib import Path
sys.path.append('$INSTALL_DIR')

try:
    import manager
except ImportError as e:
    print('Failed to import manager.py:', e)
    sys.exit(1)

caddyfile_path = Path('$CADDYFILE')
if not caddyfile_path.exists():
    print('No existing Caddyfile found at $CADDYFILE. Skipping migration.')
    sys.exit(0)

# Read content
content = caddyfile_path.read_text(encoding='utf-8')

if 'import $VHOSTS_DIR/enabled/*.caddy' in content:
    print('Caddyfile already contains split vhosts import. Skipping split migration.')
    sys.exit(0)

# Back up the original file
backup_path = caddyfile_path.with_suffix('.bak.original')
backup_path.write_text(content, encoding='utf-8')
print(f'Original Caddyfile backed up to: {backup_path}')

# Parse & Split
prefix, blocks = manager.split_top_level_blocks(content)
manager.ensure_vhost_dirs()

migrated = 0
for block in blocks:
    header = block['header']
    if manager.is_global_or_snippet(header):
        prefix.append(block['source'])
        continue

    try:
        filename = manager.route_filename(header)
        target = manager.ENABLED_DIR / filename
        target.write_text(block['source'].rstrip() + '\n', encoding='utf-8')
        print(f' -> Migrated site: {header} -> {target.name}')
        migrated += 1
    except Exception as e:
        print(f'Failed to migrate site block {header}: {e}')
        prefix.append(block['source'])

# Update Caddyfile to include import
new_caddyfile = []
prefix_str = '\n\n'.join(prefix).strip()
if prefix_str:
    new_caddyfile.append(prefix_str)
new_caddyfile.append('import $VHOSTS_DIR/enabled/*.caddy')

caddyfile_path.write_text('\n\n'.join(new_caddyfile).strip() + '\n', encoding='utf-8')
print(f'Migration complete! Split {migrated} site(s). Caddyfile now references vhosts directory.')
"
else
  # Ensure the vhost folders are created anyway
  echo "Ensuring vhost folders exist..."
  mkdir -p "$VHOSTS_DIR/enabled"
  mkdir -p "$VHOSTS_DIR/disabled"
fi

# 8. Create systemd service
echo -e "\nSetting up systemd service..."
cat << SVCEOF > /etc/systemd/system/caddedit.service
[Unit]
Description=Caddedit - Caddyfile GUI Editor
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/.venv/bin/uvicorn manager:app --host 0.0.0.0 --port $PORT
Restart=always
RestartSec=5
EnvironmentFile=$INSTALL_DIR/.env

[Install]
WantedBy=multi-user.target
SVCEOF

# Enable and start the service
systemctl daemon-reload
systemctl enable caddedit
systemctl restart caddedit

# 9. Install the caddedit CLI (talks to the API above - separate from the server itself)
echo -e "\n${GREEN}=== Caddedit CLI ===${NC}"
read -p "Install the 'caddedit' CLI client to $CLI_BIN? [Y/n]: " CLI_INPUT
CLI_INPUT=${CLI_INPUT:-Y}
if [[ "$CLI_INPUT" =~ ^[Yy]$ ]]; then
  curl -sSL "$BASE_URL/caddedit" -o "$CLI_BIN"
  chmod +x "$CLI_BIN"

  # Pre-configure the CLI for the root user so it works out of the box
  # from this server's own shell. Anyone else can run
  # `caddedit config set-url` / `set-password` to point it elsewhere.
  mkdir -p /root/.config/caddedit
  cat << CLIEOF > /root/.config/caddedit/cli.json
{
  "url": "http://127.0.0.1:$PORT",
  "password": "$PASSWORD"
}
CLIEOF
  chmod 600 /root/.config/caddedit/cli.json

  echo -e "Installed CLI: ${YELLOW}$CLI_BIN${NC}"
  echo -e "Pre-configured for root - try: ${YELLOW}caddedit list${NC}"
else
  echo "Skipped CLI install."
fi

echo -e "\n${GREEN}=== Caddedit Installed Successfully! ===${NC}"
echo -e "Access URL:       ${YELLOW}http://$(curl -s https://ifconfig.me || echo "your_server_ip"):$PORT${NC}"
echo -e "Unlock Password:  ${YELLOW}$PASSWORD${NC}"
echo -e "AI Features:      ${YELLOW}$( [ "$DISABLE_AI" = "true" ] && echo "Disabled / Hidden" || echo "Enabled" )${NC}"
echo -e "Caddedit files:   $INSTALL_DIR"
echo -e "Systemd service:  systemctl status caddedit"
if [[ "$CLI_INPUT" =~ ^[Yy]$ ]]; then
  echo -e "CLI tool:         caddedit list   (run as root, or run 'caddedit config set-url/set-password' as any other user)"
fi
echo -e "========================================="