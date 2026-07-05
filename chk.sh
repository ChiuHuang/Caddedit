#!/bin/bash
# ====== 哪吒漏洞入侵 - 全自动清理脚本 v2.6.3 ======
# 用法: bash nezha_auto_clean.sh
# 或一行执行: curl -sL <url> | bash

# set -e  # 部分机器 docker 未运行会误触发退出，改用逐行 || true 保护

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log() { echo -e "${GREEN}[+]${NC} $1"; }
warn() { echo -e "${YELLOW}[!]${NC} $1"; }
fail() { echo -e "${RED}[-]${NC} $1"; }

echo "=========================================="
echo " 哪吒后门全自动清理 v2.6.4"
echo "=========================================="
echo ""
echo -e "${YELLOW}⚠️  警告：此脚本将清空 SSH 公钥、清理定时任务！${NC}"
echo -e "${YELLOW}⚠️  注意：本脚本将彻底卸载哪吒监控 (Nezha) 及 Agent，请事后重新安装！${NC}"
echo -e "${YELLOW}⚠️  请确保你已有其他登录方式（密码/控制台），否则可能失联！${NC}"
echo ""
read -p "确认已了解风险并继续？(y/N): " confirm < /dev/tty
if [ "$confirm" != "y" ] && [ "$confirm" != "Y" ]; then
    echo "已取消"
    exit 0
fi
echo ""

# ---- 0. 解除 LD_PRELOAD 劫持（必须最先执行，否则 ps/ss/find 可能被欺骗） ----
log "0/10 解除 LD_PRELOAD 劫持..."
LD_DETECTED=0

# 检查全局 ld.so.preload 配置文件
if [ -f /etc/ld.so.preload ]; then
    warn "发现 /etc/ld.so.preload，正在备份并清空..."
    cp /etc/ld.so.preload "/etc/ld.so.preload.bak.$(date +%s)" 2>/dev/null
    > /etc/ld.so.preload
    LD_DETECTED=1
fi

# 检查当前 Shell 环境变量
if [ -n "$LD_PRELOAD" ]; then
    warn "检测到 LD_PRELOAD 环境变量=$LD_PRELOAD，正在解除..."
    unset LD_PRELOAD
    LD_DETECTED=1
fi

# 递归检查子进程环境（攻击者可能在父进程注入）
for pid_dir in /proc/*/environ; do
    [ -f "$pid_dir" ] || continue
    if grep -q "LD_PRELOAD" "$pid_dir" 2>/dev/null; then
        warn "  发现进程含有 LD_PRELOAD 劫持: $(basename "$(dirname "$pid_dir")")"
    fi
done 2>/dev/null || true

# 若曾检测到劫持，提示重新登录以清除进程内存中的恶意 so 缓存
if [ "$LD_DETECTED" = "1" ]; then
    warn "  LD_PRELOAD 已被清空，但已运行的 shell 进程内存中可能仍缓存被劫持的 so"
    warn "  建议脚本执行完毕后 exec bash 或重新登录，否则后续 ps/find 仍可能被欺骗"
fi

log "  完成"

# ---- 1. 杀挖矿进程 ----
log "1/10 清理挖矿进程..."

# 先停挖矿相关 systemd 服务（防止 auto-restart 复活）
for miner_svc in xmrig c3pool_miner; do
    systemctl stop "$miner_svc" 2>/dev/null || true
    systemctl disable "$miner_svc" 2>/dev/null || true
done

# 方法A: 按已知名称杀
for name in xmrig xmrig-daemon stratum c3pool kdevtmpfsi kinsing sys_root_svc; do
    pkill -9 -f "$name" 2>/dev/null || true
done

# 方法B: 按矿池专属端口杀（已移除 8080/8888/9999 避免误杀 Web 服务）
MINING_PORTS="3333|4444|5555|14444|14433"
ss -tnp 2>/dev/null | grep -E ":($MINING_PORTS)" | grep -oP 'pid=\K\d+' | sort -u | while read pid; do
    [ -n "$pid" ] && kill -9 "$pid" 2>/dev/null || true
done

# 方法C: 搜 xmrig 二进制（按 magic bytes 或特征路径），找到后杀父进程
# 使用进程替换 < <(find ...) 避免管道子 Shell 的变量作用域问题
while read -r f; do
    if strings "$f" 2>/dev/null | grep -qi "xmrig\|donate-level\|cryptonight\|randomx"; then
        warn "  发现挖矿文件: $f"
        lsof "$f" 2>/dev/null | awk 'NR>1{print $2}' | sort -u | while read pid; do
            kill -9 "$pid" 2>/dev/null || true
        done
        rm -rf "$f" 2>/dev/null
    fi
done < <(find /tmp /dev/shm /var/tmp /root -type f \( -name "xmrig*" -o -name "config.json" \) 2>/dev/null)

# 方法D: 检测连接已知恶意 C2 的进程并杀
MALICIOUS_C2="data.sh0.cn|sh0.cn|c2tools.caoyuanke.org|139.199.221.213|212.83.185.19"
ss -tnp 2>/dev/null | grep -E "($MALICIOUS_C2)" | grep -oP 'pid=\K\d+' | sort -u | while read pid; do
    [ -n "$pid" ] && warn "  发现连接恶意 C2 的进程 PID=$pid" && kill -9 "$pid" 2>/dev/null || true
done

rm -rf /root/c3pool /tmp/.sys_root_svc /tmp/.sys_*_svc* /tmp/.X11-unix /tmp/.sys_lock /tmp/.sys_id /tmp/.cmd_* 2>/dev/null || true
log "  完成"

# ---- 2. 停止并删除恶意 systemd 服务 ----
log "2/10 清理恶意服务..."

# 精确匹配已知恶意服务名（包含正版 nezha-agent/nazha-agent 无后缀名，运行时请确认不需要哪吒监控）
MALICIOUS_SERVICES=(
    nezha-agent nazha-agent nezha-dashboard
    pfpfybsmne V2bX
    c3pool_miner xmrig
    history_check networktraffic
    BT-FirewallServices site_total
)
for svc in "${MALICIOUS_SERVICES[@]}"; do
    if systemctl is-active "$svc" &>/dev/null 2>&1; then
        systemctl stop "$svc" 2>/dev/null || true
        warn "  已停止: $svc"
    fi
    if systemctl is-enabled "$svc" &>/dev/null 2>&1; then
        systemctl disable "$svc" 2>/dev/null || true
    fi
    rm -f "/etc/systemd/system/${svc}.service" 2>/dev/null
    rm -f "/etc/systemd/system/multi-user.target.wants/${svc}.service" 2>/dev/null
    rm -f "/lib/systemd/system/${svc}.service" 2>/dev/null
done

# 通配匹配：nezha-agent-xxxxxx / nazha-agent-xxxxxx 等服务名带 hash 后缀的变体（攻击者特征）
for suffix_svc in $(systemctl list-unit-files --all 2>/dev/null | grep -iE "nezha-agent-|nazha-agent-" | awk '{print $1}' || true); do
    systemctl stop "$suffix_svc" 2>/dev/null || true
    systemctl disable "$suffix_svc" 2>/dev/null || true
    rm -f "/etc/systemd/system/${suffix_svc}" "/etc/systemd/system/multi-user.target.wants/${suffix_svc}" "/lib/systemd/system/${suffix_svc}" 2>/dev/null
    warn "  已清理: $suffix_svc"
done

systemctl daemon-reload 2>/dev/null || true
# 删除 /opt/nezha 整个目录树（含 agent config 等）
log "  完成"


# ---- 4. 清理定时任务（仅删除极高置信度的恶意特征，保留合法业务） ----
log "4/10 清理定时任务..."

# 仅匹配明确恶意下载/执行特征，不含宽泛词，防止误杀宝塔/1Panel/证书续期等合法任务
MALICIOUS_CRON_PATTERN="curl.*\|.*sh|wget.*\|.*sh|base64.*-d|/tmp/\.sys_|c3pool|xmrig|band\.png|nvm\.exe"

for user in $(cut -f1 -d: /etc/passwd); do
    # 先备份原 crontab
    crontab -u "$user" -l > "/tmp/crontab_bak_${user}_$(date +%s)" 2>/dev/null || true
    # 过滤恶意特征并重新写入
    crontab -u "$user" -l 2>/dev/null | grep -viE "$MALICIOUS_CRON_PATTERN" | crontab -u "$user" - 2>/dev/null || true
done

# 清理 /etc/cron.d 中的可疑文件
if [ -d /etc/cron.d ]; then
    for f in /etc/cron.d/*; do
        [ -f "$f" ] || continue
        if grep -qiE "$MALICIOUS_CRON_PATTERN" "$f" 2>/dev/null; then
            warn "  删除可疑文件: $f"
            rm -f "$f"
        fi
    done
fi
log "  完成"

# ---- 5. 清理 Shell 配置文件后门（防止复活） ----
log "5/10 清理 Shell 配置文件后门..."

# 攻击者常在 .bashrc / .profile / etc/profile 末尾注入恶意代码，每次登录即复活
PROFILE_FILES="/root/.bashrc /root/.profile /etc/profile"
SHELL_BACKDOOR_PATTERN="curl.*\|.*sh|wget.*\|.*sh|base64.*-d|/tmp/\.sys_|c3pool|xmrig|band\.png|nvm\.exe"

for file in $PROFILE_FILES; do
    if [ -f "$file" ]; then
        if grep -qiE "$SHELL_BACKDOOR_PATTERN" "$file" 2>/dev/null; then
            warn "  发现 $file 含有恶意注入，正在备份并清理..."
            cp "$file" "$file.bak.$(date +%s)" 2>/dev/null
            sed -i "/$SHELL_BACKDOOR_PATTERN/d" "$file" 2>/dev/null
        fi
    fi
done

# 检查所有用户家目录下的 .bashrc 和 .profile
for homedir in /home/* /root; do
    for rc in .bashrc .profile .bash_profile; do
        [ -f "$homedir/$rc" ] || continue
        if grep -qiE "$SHELL_BACKDOOR_PATTERN" "$homedir/$rc" 2>/dev/null; then
            warn "  发现 $homedir/$rc 含有恶意注入，正在备份并清理..."
            cp "$homedir/$rc" "$homedir/$rc.bak.$(date +%s)" 2>/dev/null
            sed -i "/$SHELL_BACKDOOR_PATTERN/d" "$homedir/$rc" 2>/dev/null
        fi
    done
done
log "  完成"

# ---- 6. 清理恶意 Docker 容器 ----
log "6/10 清理恶意 Docker 容器..."
if command -v docker &>/dev/null; then
    # 已知流量套利容器
    MALICIOUS_CONTAINERS="pawns honeygain repocket tm traffmonetizer peer2profit earnapp packetstream"
    for c in $MALICIOUS_CONTAINERS; do
        docker stop "$c" 2>/dev/null || true
        docker rm "$c" 2>/dev/null || true
    done

    # 搜索其他可疑容器
    docker ps -a --format '{{.Names}}' 2>/dev/null | while read name; do
        [ -z "$name" ] && continue
        echo "$name" | grep -qiE "pawns|honeygain|repocket|traff|earn|proxy|profit|packet|iproyal" && {
            warn "  可疑容器: $name"
            docker stop "$name" 2>/dev/null || true
            docker rm "$name" 2>/dev/null || true
        } || true
    done || true

    # 删除已知恶意镜像
    MALICIOUS_IMAGES="iproyal/pawns-cli traffmonetizer/cli_v2 lswl/vertex lswl/vertex-base honeygain/honeygain repocket/repocket"
    for img in $MALICIOUS_IMAGES; do
        docker rmi "$img" 2>/dev/null || true
    done
fi
log "  完成"

# ---- 7. 删除已知恶意文件 ----
log "7/10 清理恶意文件残留..."

# 先杀内网穿透/跳板代理工具进程（攻击者用服务器当肉鸡跳板）
for proxy in frpc frps chashell; do
    pkill -9 "$proxy" 2>/dev/null || true
done

# SHA256 匹配已知恶意 nezha-agent 二进制
KNOWN_EVIL_SHA256="e7260c1b6a0e932a28e36e835f15b01f2109fdce6f835b6fec74594a46fd94f0"
for suspect in /opt/nezha/agent/nezha-agent /opt/nazha/agent/nezha-agent /usr/local/bin/nezha-agent /tmp/nezha-agent /root/nezha-agent; do
    if [ -f "$suspect" ] && command -v sha256sum &>/dev/null; then
        h=$(sha256sum "$suspect" 2>/dev/null | awk '{print $1}')
        if [ "$h" = "$KNOWN_EVIL_SHA256" ]; then
            warn "  发现恶意 nezha-agent (SHA256 匹配): $suspect"
            rm -f "$suspect"
        fi
    fi
done

rm -rf \
    /root/nezha* \
    /root/c3pool \
    /usr/local/bin/xmrig \
    /home/testnezha \
    /var/lib/sudo/lectured/testnezha \
    /tmp/frpc /tmp/frps /tmp/chashell \
    /usr/local/bin/frpc /usr/local/bin/frps /usr/local/bin/chashell \
    /tmp/band.png \
    /tmp/nvm.exe \
    /opt/band.png \
    /opt/nvm.exe \
    /tmp/.sys_*_svc* \
    /tmp/.sys_lock \
    /tmp/.sys_id \
    /tmp/.cmd_* \
    /tmp/.X11-unix \
    /tmp/.ICE-unix \
    /dev/shm/networktraffic \
    /etc/history_check \
    /usr/local/bin/history_check \
    /usr/local/bin/nazha* \
    /opt/nezha* \
    /opt/nazha* \
    2>/dev/null || true
log "  完成"

# ---- 8. 清理 Docker 悬空镜像 ----
log "8/10 清理 Docker 悬空镜像..."
if command -v docker &>/dev/null; then
    docker image prune -f 2>/dev/null || true
fi
log "  完成"

# ---- 9. 最终验证 ----
log "9/10 最终验证..."

MINER_COUNT=$(ps aux | grep -iE "xmrig|miner|c3pool|kdevtmpfsi|kinsing" | grep -v grep | wc -l | tr -d '[:space:]')
CRON_COUNT=$(crontab -l 2>/dev/null | grep -viE "^#" | grep -viE "^$" | grep -ciE "$MALICIOUS_CRON_PATTERN" | tr -d '\n' || echo 0)
AUTH_COUNT=$(grep -c . /root/.ssh/authorized_keys 2>/dev/null || echo 0)
AUTH_COUNT=$(echo "$AUTH_COUNT" | tr -d '\n')
AUTH_COUNT=${AUTH_COUNT:-0}
SVC_LEFT=$(systemctl list-units --all 2>/dev/null | grep -iE "nezha|nazha|pfpfybsmne|V2bX|c3pool" | wc -l | tr -d '[:space:]' || echo 0)
C2_CONN=$(ss -tnp 2>/dev/null | grep -cE "data.sh0.cn|sh0.cn|c2tools.caoyuanke.org|212.83.185.19" | tr -d '\n'); C2_CONN=${C2_CONN:-0}
LD_PRELOAD_OK=0; [ -f /etc/ld.so.preload ] && [ -s /etc/ld.so.preload ] && LD_PRELOAD_OK=1
SHELL_RC_LEFT=$(for f in /root/.bashrc /root/.profile /etc/profile; do [ -f "$f" ] && grep -ciE "$SHELL_BACKDOOR_PATTERN" "$f" 2>/dev/null; done | paste -sd+ 2>/dev/null | bc 2>/dev/null)
SHELL_RC_LEFT=$(echo "${SHELL_RC_LEFT:-0}" | tr -d '\n')
FRP_LEFT=$(ps aux | grep -iE "frpc|frps|chashell" | grep -v grep | wc -l | tr -d '[:space:]')

echo ""
echo "=========================================="
echo " 清理报告"
echo "=========================================="
echo " LD_PRELOAD劫持: $LD_PRELOAD_OK  (应为 0)"
echo " 挖矿进程残留 : $MINER_COUNT  (应为 0)"
echo " 恶意cron残留 : $CRON_COUNT  (应为 0)"
echo " Shell注入残留 : $SHELL_RC_LEFT  (应为 0)"
echo " SSH 公钥数量  : $AUTH_COUNT  (应为 0)"
echo " 恶意服务残留 : $SVC_LEFT  (应为 0)"
echo " 跳板代理残留 : $FRP_LEFT  (应为 0)"
echo " C2连接残留   : $C2_CONN  (应为 0)"
echo "=========================================="

if [ "${LD_PRELOAD_OK:-0}" -eq 0 ] && [ "${MINER_COUNT:-0}" -eq 0 ] && [ "${CRON_COUNT:-0}" -eq 0 ] && [ "${SHELL_RC_LEFT:-0}" -eq 0 ] && [ "${AUTH_COUNT:-0}" -eq 0 ] && [ "${SVC_LEFT:-0}" -eq 0 ] && [ "${FRP_LEFT:-0}" -eq 0 ] && [ "${C2_CONN:-0}" -eq 0 ]; then
    echo -e "${GREEN} 状态: 清理成功！${NC}"
else
    echo -e "${RED} 状态: 仍有残留，请手动检查${NC}"
fi

echo ""
echo " 别忘了改密码: passwd"
echo " 轮换所有 API Key（对象存储/支付/AI 等）"
echo "=========================================="